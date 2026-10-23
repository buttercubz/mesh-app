package com.convlink.voice.engine.audio

import com.convlink.voice.core.FfiAudioCommand
import com.convlink.voice.core.FfiAudioIoState
import com.convlink.voice.core.FfiAudioRoute
import com.convlink.voice.core.FfiAudioSessionKind
import com.convlink.voice.core.FfiInterruptReason
import com.convlink.voice.core.FfiRoutePreference
import com.convlink.voice.engine.diag.DiagNote

/**
 * The Android audio session (DESIGN §3.2, §10.2.2): the gesture opens the session and the
 * Oboe streams with capture muted; the core's `StartCapture` only connects the sink, so a
 * late or duplicated `StartCapture` is harmless. Stream errors (`onErrorAfterClose`) and
 * MuteWatchdog stalls reopen the streams; a failed reopen reports `CaptureLost` /
 * `RenderLost` (ladder L2/L3) and the core answers with `OutputOnlyFallback`.
 *
 * IO health is always reported to the core, also when the gesture (not `OpenSession`)
 * opened the streams: the core's `captureOk` (and the ladder back to L0 after a
 * `resumeMic`) depends only on a `Running` / `Restored` report. A gesture-opened session
 * is reported healthy when the core's `OpenSession` / `StartCapture` for that gesture
 * arrives (so the report follows the command, never precedes it), and duplicate reports
 * are suppressed.
 *
 * Every method is thread-safe. [onMain] runs a block synchronously on the main thread
 * (AudioManager lives there); route and call callbacks arrive on the main thread and are
 * moved off it with [dispatch] (the audio thread), so the main thread never waits for this
 * object's lock while a holder of the lock waits for the main thread. Tests pass direct
 * runners.
 */
class PlatformAudio(
    private val io: AudioIo,
    private val strategy: AudioSessionStrategy?,
    private val callbacks: Callbacks,
    private val onMain: (() -> Unit) -> Unit = { it() },
    private val clock: () -> Long = System::currentTimeMillis,
    private val dispatch: (() -> Unit) -> Unit = { it() },
) {
    interface Callbacks {
        fun audioIo(state: FfiAudioIoState)
        fun audioRoute(route: FfiAudioRoute)
        fun onCall(active: Boolean)
        fun log(msg: String)

        /** Diagnostics notes (ridelog/1): stream opens, errors, focus, stalls. */
        fun diag(note: DiagNote) {}
    }

    private data class Streams(val capture: Boolean, val render: Boolean, val bypassVp: Boolean)

    private val lock = Any()
    private var streams: Streams? = null
    private var sessionActive = false
    private var captureOn = false
    private var lost = false
    private var outputOnly = false
    /** Last state reported to the core (null: nothing yet). */
    private var reported: FfiAudioIoState? = null
    private var preference = FfiRoutePreference.AUTO
    private var inputDevice = 0
    private var outputDevice = 0
    private val watchdog = MuteWatchdog()

    init {
        io.onStreamError = { input, code -> streamError(input, code) }
        strategy?.listener = object : AudioRouteController.Listener {
            override fun onRoute(route: FfiAudioRoute, inputDeviceId: Int, outputDeviceId: Int) {
                dispatch { routeChanged(route, inputDeviceId, outputDeviceId) }
            }

            override fun onCall(active: Boolean) = dispatch { callChanged(active) }

            override fun onFocusLost(transient: Boolean) {
                callbacks.diag(DiagNote.interruption(if (transient) "focusLossTransient" else "focusLoss", "begin"))
                callbacks.log("audio focus lost (transient=$transient)")
            }
        }
    }

    val isLive: Boolean get() = synchronized(lock) { streams != null }
    val isCapturing: Boolean get() = synchronized(lock) { captureOn }

    /**
     * Gesture-bound work (inside the tap): enters the session and opens the streams, capture
     * muted. Returns true when this call opened something (so [rollbackGesture] may undo it).
     * Throws [IllegalStateException] when the streams cannot be opened.
     */
    fun prepareGesture(plan: GesturePlan): Boolean = synchronized(lock) {
        val wanted = Streams(
            capture = plan.path != AudioPath.RENDER_ONLY,
            render = true,
            bypassVp = plan.path == AudioPath.BYPASS_VP,
        )
        if (streams == wanted && !lost) return false
        enterSession(wanted.capture)
        val r = openLocked(wanted)
        check(r == 0) { "Oboe open failed ($r)" }
        lost = false
        outputOnly = false
        true
    }

    /** Undoes [prepareGesture] when the engine refused the command. */
    fun rollbackGesture() = synchronized(lock) { closeLocked() }

    /** `PlatformHooks.audio` (engine thread). */
    fun handle(cmd: FfiAudioCommand) = synchronized(lock) {
        when (cmd) {
            is FfiAudioCommand.OpenSession -> {
                val p = cmd.profile
                val wanted = Streams(
                    capture = p.capture,
                    render = true,
                    bypassVp = p.bypassVoiceProcessing,
                )
                if (streams == null || streams!!.capture != wanted.capture) {
                    enterSession(wanted.capture)
                    val r = openLocked(wanted)
                    if (r != 0) {
                        callbacks.log("OpenSession(${p.kind}) failed: $r")
                        lost = true
                        report(if (wanted.capture) FfiAudioIoState.CaptureLost else FfiAudioIoState.RenderLost)
                    } else if (p.kind != FfiAudioSessionKind.CHECK || p.capture) {
                        reportHealthy()
                    }
                } else if (!lost) {
                    // The gesture already opened these streams (§3.2): report them now,
                    // or the core never learns that capture runs ("mic paused" forever).
                    reportHealthy()
                }
            }
            FfiAudioCommand.StartCapture -> {
                captureOn = true
                io.setCaptureEnabled(true)
                watchdog.arm(io.stats()?.captureCallbacks ?: 0, clock())
                // `resumeMic` sends only StartCapture: a capture stream the gesture
                // (re)opened is healthy, so the ladder can return to L0.
                if (streams?.capture == true && !lost && !outputOnly) reportHealthy()
            }
            FfiAudioCommand.StopCapture -> {
                captureOn = false
                io.setCaptureEnabled(false)
                watchdog.disarm()
            }
            is FfiAudioCommand.SetRoutePreference -> {
                preference = cmd.pref
                if (sessionActive) onMain { strategy?.setPreference(cmd.pref) }
            }
            is FfiAudioCommand.OutputOnlyFallback -> outputOnlyFallback(cmd.enabled)
            FfiAudioCommand.CloseSession -> {
                closeLocked()
                report(FfiAudioIoState.Stopped)
            }
        }
    }

    /** Polled by the host (≈ 250 ms): MuteWatchdog over `mesh_rt_stats` (§10.1.4). */
    fun tick() = synchronized(lock) {
        val s = streams ?: return
        if (!captureOn || !s.capture || lost) return
        val count = io.stats()?.captureCallbacks ?: return
        if (watchdog.check(count, clock())) {
            callbacks.diag(DiagNote.captureStall(watchdog.stallMs))
            callbacks.log("MuteWatchdog: capture stalled > ${watchdog.stallMs} ms")
            restartLocked()
        }
    }

    /** Closes everything (engine stop). */
    fun close() = synchronized(lock) { closeLocked() }

    // MARK: internals (all under `lock`)

    private fun enterSession(capture: Boolean) {
        if (sessionActive) return
        var ok = true
        onMain { ok = strategy?.activate(preference, capture) ?: true }
        if (!ok) callbacks.log("audio session: focus not granted; continuing on the default route")
        sessionActive = true
    }

    private fun openLocked(s: Streams): Int {
        if (streams != null) io.close()
        val r = io.open(s.capture, s.render, inputDevice, outputDevice, s.bypassVp)
        callbacks.diag(DiagNote.audioOpen(s.capture, s.render, s.bypassVp, r, inputDevice, outputDevice))
        streams = if (r == 0) s else null
        if (r == 0) {
            io.setCaptureEnabled(captureOn && s.capture)
            if (captureOn && s.capture) watchdog.arm(io.stats()?.captureCallbacks ?: 0, clock())
        }
        return r
    }

    private fun closeLocked() {
        io.close()
        if (reported !is FfiAudioIoState.CaptureLost && reported !is FfiAudioIoState.RenderLost) reported = null
        streams = null
        captureOn = false
        lost = false
        outputOnly = false
        watchdog.disarm()
        if (sessionActive) {
            sessionActive = false
            onMain { strategy?.deactivate() }
        }
    }

    private fun restartLocked() {
        val s = streams ?: lastStreams ?: return
        report(FfiAudioIoState.Restarting)
        val r = openLocked(s)
        if (r == 0) {
            report(if (lost) FfiAudioIoState.Restored else FfiAudioIoState.Running)
            lost = false
        } else {
            lost = true
            watchdog.disarm()
            report(if (s.capture) FfiAudioIoState.CaptureLost else FfiAudioIoState.RenderLost)
        }
    }

    private var lastStreams: Streams? = null

    private fun outputOnlyFallback(enabled: Boolean) {
        if (enabled == outputOnly) return
        outputOnly = enabled
        val target = if (enabled) {
            lastStreams = streams ?: lastStreams
            Streams(capture = false, render = true, bypassVp = false)
        } else {
            lastStreams ?: Streams(capture = true, render = true, bypassVp = false)
        }
        if (openLocked(target) != 0) report(FfiAudioIoState.RenderLost)
    }

    private fun streamError(input: Boolean, code: Int) {
        // Oboe calls back on its own thread after the stream closed; reopen off that thread.
        dispatch {
            synchronized(lock) {
                if (streams == null) return@synchronized
                callbacks.diag(DiagNote.streamError(input, code))
                callbacks.log("${if (input) "input" else "output"} stream error $code; reopening")
                lastStreams = streams
                restartLocked()
            }
        }
    }

    private fun routeChanged(route: FfiAudioRoute, inDev: Int, outDev: Int) {
        callbacks.diag(DiagNote.audioRoute(route.kind.name.lowercase(), route.inputRate.toInt(), headsetClass(route)))
        callbacks.audioRoute(route)
        synchronized(lock) {
            val changed = inDev != inputDevice || outDev != outputDevice
            inputDevice = inDev
            outputDevice = outDev
            // Oboe streams follow the communication device; reopen only on a real change.
            if (changed && streams != null) restartLocked()
        }
    }

    private fun callChanged(active: Boolean) {
        callbacks.diag(DiagNote.interruption("call", if (active) "begin" else "end"))
        callbacks.onCall(active)
        if (active) {
            report(FfiAudioIoState.Interrupted(FfiInterruptReason.CALL))
        } else {
            synchronized(lock) { if (streams != null) restartLocked() }
        }
    }

    /**
     * Reports `Running`, or `Restored` after a reported loss, unless the core already
     * knows the IO is healthy.
     */
    private fun reportHealthy() {
        val last = reported
        if (last == FfiAudioIoState.Running || last == FfiAudioIoState.Restored) return
        val wasLost = last == FfiAudioIoState.CaptureLost || last == FfiAudioIoState.RenderLost
        report(if (wasLost) FfiAudioIoState.Restored else FfiAudioIoState.Running)
    }

    private fun report(state: FfiAudioIoState) {
        reported = state
        callbacks.audioIo(state)
    }

    /** A coarse headset class for diagnostics: never the device name. */
    private fun headsetClass(route: FfiAudioRoute): String = when {
        route.output.name.contains("CAR", ignoreCase = true) -> "car"
        route.output.name.startsWith("HFP") || route.output == com.convlink.voice.core.FfiAudioRouteKind.LE_AUDIO -> "bluetooth"
        route.output.name.contains("WIRED", ignoreCase = true) -> "wired"
        route.headsetPresent -> "unknown"
        else -> "none"
    }
}
