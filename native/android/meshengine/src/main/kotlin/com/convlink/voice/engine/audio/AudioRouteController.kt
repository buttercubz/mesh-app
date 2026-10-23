package com.convlink.voice.engine.audio

import android.content.Context
import android.media.AudioAttributes
import android.media.AudioDeviceCallback
import android.media.AudioDeviceInfo
import android.media.AudioFocusRequest
import android.media.AudioManager
import android.os.Handler
import android.os.Looper
import android.os.SystemClock
import com.convlink.voice.core.FfiAudioRoute
import com.convlink.voice.core.FfiRoutePreference
import java.util.concurrent.Executor

/**
 * Communication route (DESIGN §10.2.2): `MODE_IN_COMMUNICATION`, `setCommunicationDevice`
 * per [RouteSelector], `OnCommunicationDeviceChangedListener` (the session proceeds on the
 * default route while the requested device takes up to 30 s to come up), and
 * `clearCommunicationDevice()` on exit.
 *
 * Audio focus (2026-10-23 field-test deviation from §10.2.2, which held `AUDIOFOCUS_GAIN`
 * for the whole ride): riders play music, and a permanent `AUDIOFOCUS_GAIN` makes the music
 * app pause/stop. Entering the session therefore requests no focus; [setVoiceActive] takes
 * `AUDIOFOCUS_GAIN_TRANSIENT_MAY_DUCK` (`USAGE_VOICE_COMMUNICATION`) only while a peer's
 * voice plays and releases it [DuckingFocus.RELEASE_HANGOVER_MS] after the last voice, so
 * the music ducks while someone talks and comes back between overs (Cardo-like). Losing
 * focus to a music app (`AUDIOFOCUS_LOSS*`) never stops capture or playback: it is only
 * reported (Android does not enforce focus; the Oboe streams keep running).
 *
 * A real call takes SCO (the dialer has priority): [Listener.onCall] reports it and
 * the route is re-applied after the call.
 */
class AudioRouteController(context: Context, private val handler: Handler = Handler(Looper.getMainLooper())) {
    interface Listener {
        fun onRoute(route: FfiAudioRoute, inputDeviceId: Int, outputDeviceId: Int)
        fun onCall(active: Boolean)
        fun onFocusLost(transient: Boolean)
    }

    private val am = context.getSystemService(AudioManager::class.java)
    private val executor = Executor { handler.post(it) }
    private val ducking = DuckingFocus(
        object : DuckingFocus.Port {
            override fun request(): Boolean = am.requestAudioFocus(duckRequest) == AudioManager.AUDIOFOCUS_REQUEST_GRANTED
            override fun abandon() {
                am.abandonAudioFocusRequest(duckRequest)
            }
        },
    )
    private var active = false
    private var pref = FfiRoutePreference.AUTO
    private var capture = true
    private var requestedAt = 0L
    private var onCall = false

    @Volatile
    var listener: Listener? = null

    private val deviceListener = AudioManager.OnCommunicationDeviceChangedListener { report() }

    private val modeListener = AudioManager.OnModeChangedListener { mode ->
        val call = mode == AudioManager.MODE_IN_CALL || mode == AudioManager.MODE_RINGTONE ||
            mode == AudioManager.MODE_CALL_SCREENING
        if (call != onCall) {
            onCall = call
            listener?.onCall(call)
            if (!call && active) apply()
        }
    }

    private val deviceCallback = object : AudioDeviceCallback() {
        override fun onAudioDevicesAdded(added: Array<out AudioDeviceInfo>) {
            if (active) apply()
        }

        override fun onAudioDevicesRemoved(removed: Array<out AudioDeviceInfo>) {
            if (active) apply()
        }
    }

    private val focusListener = AudioManager.OnAudioFocusChangeListener { change ->
        // Report only: capture and playback keep running whoever holds focus.
        DuckingFocus.lossKind(change)?.let { transient ->
            ducking.lost()
            listener?.onFocusLost(transient)
        }
    }

    private val duckRequest: AudioFocusRequest by lazy {
        AudioFocusRequest.Builder(AudioManager.AUDIOFOCUS_GAIN_TRANSIENT_MAY_DUCK)
            .setAudioAttributes(
                AudioAttributes.Builder()
                    .setUsage(AudioAttributes.USAGE_VOICE_COMMUNICATION)
                    .setContentType(AudioAttributes.CONTENT_TYPE_SPEECH)
                    .build(),
            )
            .setOnAudioFocusChangeListener(focusListener, handler)
            .setAcceptsDelayedFocusGain(false)
            .setWillPauseWhenDucked(false)
            .build()
    }

    private val releaseCheck = Runnable { ducking.tick(SystemClock.elapsedRealtime()) }

    val isActive: Boolean get() = active

    /** Enters the communication mode and applies the route. Main thread. */
    fun enter(preference: FfiRoutePreference, withCapture: Boolean): Boolean {
        pref = preference
        capture = withCapture
        if (!active) {
            am.mode = AudioManager.MODE_IN_COMMUNICATION
            // No focus here: holding focus for the whole ride pauses the rider's music.
            am.addOnCommunicationDeviceChangedListener(executor, deviceListener)
            am.addOnModeChangedListener(executor, modeListener)
            am.registerAudioDeviceCallback(deviceCallback, handler)
            active = true
        }
        apply()
        return true
    }

    /**
     * A peer's voice started (true) or stopped (false) playing: ducks other audio while it
     * plays (see the class doc). Main thread. Driven by the playout owner (PlatformAudio).
     */
    fun setVoiceActive(voice: Boolean) {
        if (!active) return
        val now = SystemClock.elapsedRealtime()
        ducking.setVoiceActive(voice, now)
        handler.removeCallbacks(releaseCheck)
        if (!voice) handler.postDelayed(releaseCheck, DuckingFocus.RELEASE_HANGOVER_MS)
    }

    fun setPreference(preference: FfiRoutePreference) {
        pref = preference
        if (active) apply()
    }

    /** Leaves the communication mode (clears the device, abandons focus). Main thread. */
    fun exit() {
        if (!active) return
        active = false
        am.clearCommunicationDevice()
        am.removeOnCommunicationDeviceChangedListener(deviceListener)
        am.removeOnModeChangedListener(modeListener)
        am.unregisterAudioDeviceCallback(deviceCallback)
        handler.removeCallbacks(releaseCheck)
        ducking.release()
        am.mode = AudioManager.MODE_NORMAL
    }

    private fun devices(): List<Pair<RouteDevice, AudioDeviceInfo>> =
        am.availableCommunicationDevices.map { RouteDevice(it.id, it.type, it.sampleRates) to it }

    private fun apply() {
        val all = devices()
        val target = RouteSelector.choose(all.map { it.first }, pref)
        val info = all.firstOrNull { it.first == target }?.second
        val current = am.communicationDevice
        if (info != null && current?.id != info.id) {
            requestedAt = SystemClock.elapsedRealtime()
            if (!am.setCommunicationDevice(info)) requestedAt = 0
            // The listener reports when it switched; until then the default route plays.
            handler.postDelayed({ if (active) report() }, SWITCH_WAIT_MS)
        }
        report()
    }

    private fun report() {
        if (!active) return
        val all = devices().map { it.first }
        val dev = am.communicationDevice?.let { RouteDevice(it.id, it.type, it.sampleRates) }
        val id = dev?.id ?: 0
        listener?.onRoute(RouteSelector.route(dev, all, capture), id, id)
    }

    /** True while the requested device is still within its 30 s switch window. */
    fun waitingForDevice(now: Long = SystemClock.elapsedRealtime()): Boolean =
        requestedAt != 0L && now - requestedAt < SWITCH_WAIT_MS

    companion object {
        const val SWITCH_WAIT_MS = 30_000L
    }
}

/**
 * Pure ducking-focus policy (unit-tested): request transient may-duck focus when voice
 * starts, keep it across short gaps, abandon it [RELEASE_HANGOVER_MS] after the last voice.
 * A focus loss forgets the grant (the next voice re-requests it) and never stops audio.
 */
class DuckingFocus(private val port: Port) {
    interface Port {
        fun request(): Boolean
        fun abandon()
    }

    var held = false
        private set
    private var voice = false
    private var lastVoiceEnd = 0L

    fun setVoiceActive(active: Boolean, now: Long) {
        voice = active
        if (active) {
            if (!held) held = port.request()
        } else {
            lastVoiceEnd = now
        }
    }

    /** Releases the focus once the hangover after the last voice has elapsed. */
    fun tick(now: Long) {
        if (held && !voice && now - lastVoiceEnd >= RELEASE_HANGOVER_MS) release()
    }

    /** Another app took the focus: forget the grant; audio keeps running. */
    fun lost() {
        held = false
    }

    fun release() {
        if (held) port.abandon()
        held = false
        voice = false
    }

    companion object {
        /** Keeps music ducked between consecutive overs. [TUNE] */
        const val RELEASE_HANGOVER_MS = 1_500L

        /** `AUDIOFOCUS_LOSS*` → transient flag; null for gains and duck-only changes. */
        fun lossKind(change: Int): Boolean? = when (change) {
            AudioManager.AUDIOFOCUS_LOSS -> false
            AudioManager.AUDIOFOCUS_LOSS_TRANSIENT -> true
            // We are only a voice overlay: being asked to duck is ignored (never paused).
            else -> null
        }
    }
}
