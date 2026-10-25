package com.convlink.voice.engine

import android.annotation.SuppressLint
import android.app.ActivityManager
import android.content.Context
import android.os.Build
import android.os.Handler
import android.os.HandlerThread
import android.os.Looper
import android.util.Log
import com.convlink.voice.core.FfiAudioCommand
import com.convlink.voice.core.FfiAudioIoState
import com.convlink.voice.core.FfiAudioRoute
import com.convlink.voice.core.FfiDiscovery
import com.convlink.voice.core.FfiEngineConfig
import com.convlink.voice.core.FfiLinkDownReason
import com.convlink.voice.core.FfiLinkInfo
import com.convlink.voice.core.FfiLogLevel
import com.convlink.voice.core.FfiNotification
import com.convlink.voice.core.FfiPersistedEntry
import com.convlink.voice.core.FfiPlatform
import com.convlink.voice.core.FfiPlatformSample
import com.convlink.voice.core.FfiPromptRequest
import com.convlink.voice.core.FfiRadioAvail
import com.convlink.voice.core.FfiRadioCommand
import com.convlink.voice.core.FfiRuntimeStatus
import com.convlink.voice.core.FfiSendOutcome
import com.convlink.voice.core.FfiSessionStart
import com.convlink.voice.core.MeshEngine
import com.convlink.voice.core.MeshEngineInterface
import com.convlink.voice.core.PlatformHooks
import com.convlink.voice.core.apiVersion
import com.convlink.voice.core.bleConstants
import com.convlink.voice.core.protocolVersion
import com.convlink.voice.core.rtAbiVersion
import com.convlink.voice.engine.audio.AudioRouteController
import com.convlink.voice.engine.audio.AudioSessionStrategy
import com.convlink.voice.engine.audio.CommandJson
import com.convlink.voice.engine.audio.GestureCommand
import com.convlink.voice.engine.audio.GesturePlan
import com.convlink.voice.engine.audio.OboeBridge
import com.convlink.voice.engine.audio.PlatformAudio
import com.convlink.voice.engine.audio.PromptRenderer
import com.convlink.voice.engine.audio.RtFlags
import com.convlink.voice.engine.ble.BleConfig
import com.convlink.voice.engine.ble.BleRadio
import com.convlink.voice.engine.ble.LinkSink
import com.convlink.voice.engine.ble.MeshLink
import com.convlink.voice.engine.diag.DiagFileStore
import com.convlink.voice.engine.diag.DiagNote
import com.convlink.voice.engine.diag.DiagNoteSink
import com.convlink.voice.engine.diag.DiagPreference
import com.convlink.voice.engine.diag.DiagWriter
import com.convlink.voice.engine.keys.SecureStore
import com.convlink.voice.engine.lab.LabFileStore
import com.convlink.voice.engine.notify.EventDigest
import com.convlink.voice.engine.notify.IntercomAction
import com.convlink.voice.engine.notify.IntercomNotifications
import com.convlink.voice.engine.notify.IntercomState
import com.convlink.voice.engine.notify.NotificationCatalog
import com.convlink.voice.engine.aware.AndroidAwareOs
import com.convlink.voice.engine.aware.AwareDriver
import com.convlink.voice.engine.aware.AwareSupport
import com.convlink.voice.engine.radio.RadioStateMonitor
import com.convlink.voice.engine.sensors.DeviceSampler
import com.convlink.voice.engine.sensors.LocationSampler
import kotlinx.coroutines.channels.BufferOverflow
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.flow.asSharedFlow
import org.json.JSONObject
import java.security.SecureRandom

/**
 * The platform side the engine calls into (`PlatformHooks`, contract C4), split from
 * [EngineHost] so it can be driven by the real Rust engine in JVM tests with fake ports.
 * Every hook runs on the engine thread and never blocks on the main thread: radio and
 * audio commands are handed to their own threads.
 */
class EngineHooks(private val ports: Ports) : PlatformHooks {
    /** Where the hooks go (implemented by EngineHost; fakes in tests). */
    interface Ports {
        fun radio(cmd: FfiRadioCommand)
        fun linkSend(link: Int, bytes: ByteArray): FfiSendOutcome
        fun audio(cmd: FfiAudioCommand)
        fun uiEvent(json: String)
        fun persist(key: String, value: ByteArray?)
        fun notify(n: FfiNotification)
        fun labWrite(runId: String, line: String, summary: Boolean)
        fun labDelete(runId: String)
        fun renderPrompts(reqs: List<FfiPromptRequest>)
        fun log(level: FfiLogLevel, msg: String)

        /** ridelog/1 batch for segment [seg] of diagnostics session [sessionId]. */
        fun diagWrite(sessionId: String, seg: Int, lines: List<String>) {}
    }

    override fun linkSend(link: UInt, bytes: ByteArray): FfiSendOutcome = ports.linkSend(link.toInt(), bytes)

    override fun radio(cmd: FfiRadioCommand) = ports.radio(cmd)

    override fun audio(cmd: FfiAudioCommand) = ports.audio(cmd)

    override fun uiEvent(json: String) = ports.uiEvent(json)

    override fun persist(key: String, value: ByteArray?) = ports.persist(key, value)

    override fun notify(n: FfiNotification) = ports.notify(n)

    override fun labWrite(runId: String, line: String) = ports.labWrite(runId, line, false)

    override fun labWriteSummary(runId: String, line: String) = ports.labWrite(runId, line, true)

    override fun labDelete(runId: String) = ports.labDelete(runId)

    override fun renderPrompts(reqs: List<FfiPromptRequest>) = ports.renderPrompts(reqs)

    override fun log(level: FfiLogLevel, msg: String) = ports.log(level, msg)

    override fun diagWrite(sessionId: String, seg: UInt, lines: List<String>) =
        ports.diagWrite(sessionId, seg.toInt(), lines)
}

/** The drivers' [LinkSink] forwarding into the engine (contract C9 → C4). */
class EngineLinkSink(private val engine: () -> MeshEngineInterface?) : LinkSink {
    override fun linkUp(link: MeshLink, info: FfiLinkInfo) {
        engine()?.onLinkUp(link.id.toUInt(), info)
    }

    override fun linkDown(id: Int, reason: FfiLinkDownReason) {
        engine()?.onLinkDown(id.toUInt(), reason)
    }

    override fun linkData(id: Int, bytes: ByteArray) {
        engine()?.onLinkData(id.toUInt(), bytes)
    }

    override fun linkWritable(id: Int) {
        engine()?.onLinkWritable(id.toUInt())
    }

    override fun linkMetrics(id: Int, rssi: Short?, attMtu: UShort?) {
        engine()?.onLinkMetrics(id.toUInt(), rssi, attMtu)
    }

    override fun discovered(d: FfiDiscovery) {
        engine()?.onDiscovered(d)
    }

    override fun helloRead(discId: Int, bytes: ByteArray) {
        engine()?.onHelloRead(discId.toUInt(), bytes)
    }

    override fun connectFailed(discId: Int, reason: FfiLinkDownReason) {
        engine()?.onConnectFailed(discId.toUInt(), reason)
    }
}

/** Host configuration (the bridge passes app metadata; defaults suit tests). */
data class EngineHostConfig(
    val appVersion: UShort = 1u,
    val appLabel: String = "0.1.0 (1)",
    /** Notification small icon (the app's monochrome icon). */
    val smallIcon: Int = android.R.drawable.stat_sys_data_bluetooth,
    /** Use the Core-Telecom strategy when compiled in (meshengine.telecom). */
    val preferTelecom: Boolean = false,
    val sampleIntervalMs: Long = 1_000,
)

/** Errors of the host API (mirrors MeshEngineKit's EngineHostError). */
class EngineHostException(val code: String, message: String) : Exception(message)

/**
 * Owns the Rust engine (`MeshEngine`, UniFFI/JNA) and implements its platform side on
 * Android (DESIGN §3.5, §10.2): radios ([BleRadio]), audio ([PlatformAudio] over Oboe),
 * persistence ([SecureStore]), Lab files, prompts, notifications and the FGS. One instance
 * per process ([obtain]); the app-local bridge (WP-19) reaches it through
 * [MeshEngineService.LocalBinder].
 *
 * Threading: every public method may be called from any thread. Gesture-bound calls
 * ([start], [gestureCommand], [goLive], [resumeMic]) belong in the user's tap, on the
 * bridge's call thread, never on the main thread while it waits for the engine.
 */
class EngineHost private constructor(context: Context, val config: EngineHostConfig) : DiagNoteSink {
    private val app = context.applicationContext
    private val main = Handler(Looper.getMainLooper())
    private val workerThread = HandlerThread("meshengine-host").apply { start() }
    private val worker = Handler(workerThread.looper)
    private val audioThread = HandlerThread("meshengine-audio").apply { start() }
    private val audioHandler = Handler(audioThread.looper)

    val registry = LinkRegistry()
    val bleConfig: BleConfig = runCatching { BleConfig.from(bleConstants()) }.getOrDefault(BleConfig.DESIGN)
    val notifications = IntercomNotifications(app, config.smallIcon)
    val rtFlags = RtFlags { OboeBridge.setFlags(it) }

    private val secureStore = SecureStore.create(app)
    private val labStore by lazy { LabFileStore.create(app) }
    internal val diagWriter by lazy { DiagWriter(runCatching { DiagFileStore.create(app) }.getOrNull(), ::log) }
    private val digest = EventDigest()
    private val eventFlow = MutableSharedFlow<String>(extraBufferCapacity = 512, onBufferOverflow = BufferOverflow.DROP_OLDEST)
    private val routes = AudioRouteController(app, main)
    private val strategy = AudioSessionStrategy.create(app, routes, config.preferTelecom)
    private val device = DeviceSampler(app)
    private val location = LocationSampler(app)
    private val prompts by lazy {
        PromptRenderer(app, { key, pcm -> engine.onPromptPcm(key, pcm) }, { key -> log("prompt $key not rendered") })
    }
    private val radio: BleRadio by lazy {
        BleRadio(app, registry, bleConfig, onPsm = { psm -> engine.onL2capPublished(psm.toUShort()) }, log = ::log)
    }
    private val radioState = RadioStateMonitor(app) { ble, _ ->
        note(DiagNote.bt(ble.name.lowercase()))
        if (ble != FfiRadioAvail.ON) radio.radioOff()
        aware.setEnabled(awareAvail() == FfiRadioAvail.ON)
        worker.post { sampleNow() }
    }

    // Wi-Fi Aware upgrade (WP-24, Android↔Android beta): its own serial thread.
    private val awareThread by lazy { HandlerThread("meshengine-aware").apply { start() } }
    private val aware: AwareDriver by lazy {
        val h = Handler(awareThread.looper)
        AwareDriver(
            registry = registry,
            os = AndroidAwareOs(app, h, ::log),
            takePmk = { peer -> engine.takeAwarePmk(peer.toUShort()) },
            post = { h.post(it) },
            schedule = { ms, r -> h.postDelayed(r, ms) },
            now = { android.os.SystemClock.elapsedRealtime() },
            log = ::log,
        )
    }

    /**
     * Wi-Fi Aware as the core sees it: `Unsupported` unless the device has the feature and
     * the user turned the beta on in Settings; then the OS availability (airplane mode,
     * Aware service state).
     */
    private fun awareAvail(): FfiRadioAvail =
        if (AwareSupport.enabled(app)) radioState.aware else FfiRadioAvail.UNSUPPORTED
    private val audio: PlatformAudio
    private val engine: MeshEngine

    @Volatile
    private var eventListener: ((String) -> Unit)? = null

    @Volatile
    private var fgsActive = 0

    @Volatile
    private var captureLost = false

    @Volatile
    private var onCall = false

    @Volatile
    private var uiAttached = false

    @Volatile
    private var running = false

    /** Foreground-active check for gesture commands (the bridge sets "activity resumed"). */
    @Volatile
    var foregroundCheck: () -> Boolean = ::processForeground

    init {
        val api = apiVersion().toInt()
        if (api shr 8 != API_MAJOR) throw EngineHostException("apiVersionMismatch", "engine API $api")
        audio = PlatformAudio(
            io = OboeBridge,
            strategy = strategy,
            callbacks = audioCallbacks(),
            onMain = ::runOnMainUnit,
            dispatch = { audioHandler.post(it) },
        )
        val seed = ByteArray(32).also { SecureRandom().nextBytes(it) }
        val persisted = secureStore.loadAll().map { (k, v) -> FfiPersistedEntry(k, v) }
        val cfg = FfiEngineConfig(
            platform = FfiPlatform.ANDROID,
            appVersion = config.appVersion,
            rngSeed = seed,
            deviceModel = "${Build.MANUFACTURER} ${Build.MODEL}",
            deviceOs = "Android ${Build.VERSION.RELEASE}",
            appLabel = config.appLabel,
            persisted = persisted,
        )
        engine = MeshEngine(cfg, EngineHooks(ports()))
        registry.sink = EngineLinkSink { engine }
        engine.diagConfigure(DiagPreference.load(app, config.appLabel), DiagPreference.isDebuggable(app))
        note(DiagNote.os("coldStart"))
    }

    // MARK: lifecycle

    /**
     * Radios + engine thread for a group lobby, Lab mode or the Demo ride (call inside the
     * tap, §1.3 F3): `startForegroundService(connectedDevice)` then `start`. Audio stays off
     * until a gesture command. The Demo ride needs no radios and no FGS.
     */
    fun start(start: FfiSessionStart) {
        if (start !is FfiSessionStart.Demo) {
            val types = fgsTypes(FgsTypes.Stage.LOBBY)
            note(DiagNote.fgs("start", types))
            MeshEngineService.startForeground(app, types)
            main.post { radioState.start() }
            radio.handler // creates the BLE thread before the first command
        }
        try {
            engine.start(start)
        } catch (e: Exception) {
            if (start !is FfiSessionStart.Demo) MeshEngineService.stop(app)
            throw EngineHostException("engine", e.toString())
        }
        running = true
        scheduleSampling()
    }

    /**
     * Stops the session (F8). Shutdown is ordered: the core floods LEAVE and closes each
     * link after `stop`, so the radio stack and the FGS go away once the links drained
     * (bounded, [BleRadio.DRAIN_BOUND_MS]).
     */
    fun stop() {
        engine.stop()
        running = false
        audioHandler.post { audio.close() }
        main.post {
            location.stop()
            radioState.stop()
        }
        aware.stop()
        captureLost = false
        digest.reset()
        notifications.cancelAlerts()
        radio.shutdownWhenDrained { MeshEngineService.stop(app) }
    }

    val status: FfiRuntimeStatus get() = engine.status()

    // MARK: commands

    /** JSON pass-through of mesh-api `UiCommand`; rejects the ★ commands (§3.2, §11.1). */
    fun send(json: String): String {
        if (GestureCommand.requiresGesture(json)) return CommandJson.error("useGestureCommand")
        val obj = CommandJson.parse(json)
        if (obj?.optString("type") == "ptt" && obj.has("down")) rtFlags.setPttDown(obj.optBoolean("down"))
        val result = engine.uiCommand(json)
        if (obj != null && CommandJson.isOk(result)) accepted(obj, result)
        return result
    }

    /**
     * The ★ commands (goLive, resumeMic, labStart with audio ≠ off, audioCheck start):
     * refuses unless the app is foreground-active, then synchronously performs the gesture
     * work — `startForegroundService(microphone [+ location])` and opening the Oboe streams
     * with capture muted — and only then forwards the command (§3.2, §10.2.3).
     */
    fun gestureCommand(json: String): String {
        if (!foregroundCheck()) throw EngineHostException("notForeground", "gesture command outside a visible tap")
        return gestureWork(json)
    }

    fun goLive(): String = gestureCommand("""{"type":"goLive"}""")

    fun resumeMic(): String = gestureCommand("""{"type":"resumeMic"}""")

    /** PTT fast path: RT flag bit 0 at once (T8), then `{type:"ptt"}`. */
    fun ptt(down: Boolean) {
        rtFlags.setPttDown(down)
        engine.uiCommand("""{"type":"ptt","down":$down}""")
    }

    // MARK: events and UI

    /** Every UI event (JSON) from now on. */
    val events: Flow<String> get() = eventFlow.asSharedFlow()

    /** A callback for every UI event (the bridge's `notifyListeners`), or null. */
    fun setEventListener(listener: ((String) -> Unit)?) {
        eventListener = listener
    }

    /** The WebView is attached: gates snapshot emission (§11.1). */
    fun setUiAttached(attached: Boolean) {
        uiAttached = attached
        engine.setUiAttached(attached)
    }

    /** Localised notification text from the UI catalogs (null → built-in ES/EN fallback). */
    fun setNotificationCatalog(catalog: NotificationCatalog?) {
        notifications.catalog = catalog ?: NotificationCatalog.fallback(java.util.Locale.getDefault())
        refreshNotification()
    }

    fun setNotificationContentIntent(intent: android.app.PendingIntent?) {
        notifications.contentIntent = intent
    }

    data class Info(
        val apiVersion: String,
        val protocolVersion: Int,
        val rtAbiVersion: Long,
        val appVersion: String,
        /** Always "full": this library links the engine (§2.4 `getInfo`). */
        val engine: String,
        val status: String,
        val uiAttached: Boolean,
        val audioStrategy: String,
        val rtLinked: Boolean,
    )

    fun info(): Info {
        val api = apiVersion().toInt()
        val status = when (engine.status()) {
            FfiRuntimeStatus.IDLE -> "idle"
            FfiRuntimeStatus.RUNNING -> "running"
            FfiRuntimeStatus.AUDIO_CHECK -> "audioCheck"
            FfiRuntimeStatus.CRASHED -> "crashed"
        }
        return Info("${api shr 8}.${api and 0xFF}", protocolVersion().toInt(), rtAbiVersion().toLong(),
            config.appLabel, "full", status, uiAttached, strategy.name, OboeBridge.hasRt())
    }

    // MARK: notification actions and FGS

    /** Ongoing-notification actions (delivered by [MeshEngineService]). */
    fun onNotificationAction(action: IntercomAction) {
        when (action) {
            IntercomAction.MUTE -> rtFlags.setHardMute(true)
            IntercomAction.UNMUTE -> rtFlags.setHardMute(false)
            // Talk latch → mesh_rt_set_flags bit 3 (edge consumed by the engine).
            IntercomAction.TALK_LATCH -> rtFlags.latchToggle()
            IntercomAction.LEAVE -> {
                stop()
                return
            }
            // The notification tap is the while-in-use exemption: no visibility check here.
            IntercomAction.RESUME -> worker.post { runCatching { gestureWork("""{"type":"resumeMic"}""") } }
        }
        refreshNotification()
    }

    /** Current FGS types for [stage] from the grants and the session's location sharing. */
    fun fgsTypes(stage: FgsTypes.Stage): Int =
        FgsTypes.compute(stage, FgsTypes.grants(app), shareLocation = digest.shareDistance || location.isActive)

    internal fun foregroundTypesChanged(types: Int) {
        fgsActive = types
    }

    /** What the ongoing notification shows now. */
    val intercomState: IntercomState get() = digest.state(rtFlags.hardMuted, captureLost)

    private fun refreshNotification() {
        if (fgsActive != 0) main.post { notifications.update(intercomState) }
    }

    // MARK: gesture work (§3.2)

    private fun gestureWork(json: String): String {
        val gesture = GestureCommand.classify(json) ?: return forward(json)
        val plan = GesturePlan.make(gesture, digest.shareDistance)
        val previousTypes = fgsActive
        // 1. FGS with the capture types, inside the tap.
        var startedLocation = false
        if (plan.wantsLocation && !location.isActive) {
            startedLocation = runOnMainSync { location.start() } == true
        }
        val stage = if (plan.microphone) FgsTypes.Stage.LIVE else FgsTypes.Stage.LAB_NO_MIC
        val grants = FgsTypes.grants(app)
        // The exact check the FGS type and the capture depend on (first field test: "mic
        // paused" with the permission granted in Settings).
        note(DiagNote.perm("microphone", if (grants.microphone) "granted" else "denied", "check",
            "ContextCompat.checkSelfPermission(RECORD_AUDIO)"))
        val types = FgsTypes.compute(stage, grants, plan.wantsLocation && location.isActive)
        note(DiagNote.fgs("start", types))
        MeshEngineService.startForeground(app, types)
        // 2. Session + Oboe streams, capture muted.
        val opened = try {
            audio.prepareGesture(plan)
        } catch (e: IllegalStateException) {
            note(DiagNote("gestureAudio", listOf("ok" to false, "err" to "openFailed")))
            log("gesture: audio not started: ${e.message}")
            if (startedLocation) main.post { location.stop() }
            if (previousTypes != 0) MeshEngineService.startForeground(app, previousTypes)
            return CommandJson.error("audioUnavailable")
        }
        // 3. Only now the command reaches the engine.
        val result = engine.uiCommand(json)
        val obj = CommandJson.parse(json)
        if (obj != null && CommandJson.isOk(result)) {
            accepted(obj, result)
            if (plan.resume) {
                captureLost = false
                notifications.cancelAlerts()
            }
        } else {
            if (opened) audio.rollbackGesture()
            if (startedLocation) main.post { location.stop() }
            if (previousTypes != 0) MeshEngineService.startForeground(app, previousTypes)
        }
        refreshNotification()
        return result
    }

    private fun forward(json: String): String {
        val result = engine.uiCommand(json)
        CommandJson.parse(json)?.let { if (CommandJson.isOk(result)) accepted(it, result) }
        return result
    }

    /** Side effects of accepted commands the host tracks itself. */
    private fun accepted(command: JSONObject, result: String) {
        digest.commandAccepted(command)
        when (command.optString("type")) {
            "labStart" -> digest.labStarted(CommandJson.parse(result)?.optString("runId")?.ifEmpty { null })
            "setShareDistance" -> if (!command.optBoolean("enabled")) main.post { location.stop() }
            "setBuiltinMicTx" -> rtFlags.setBuiltinMicMute(!command.optBoolean("allowed", true))
        }
        refreshNotification()
    }

    // MARK: hooks

    private fun ports() = object : EngineHooks.Ports {
        override fun radio(cmd: FfiRadioCommand) {
            if (cmd is FfiRadioCommand.AwareUpgrade) aware.apply(cmd) else radio.apply(cmd)
        }

        override fun linkSend(link: Int, bytes: ByteArray) = registry.send(link, bytes)

        override fun audio(cmd: FfiAudioCommand) {
            audioHandler.post { audio.handle(cmd) }
        }

        override fun uiEvent(json: String) {
            eventFlow.tryEmit(json)
            eventListener?.invoke(json)
            if (digest.apply(json)) refreshNotification()
        }

        override fun persist(key: String, value: ByteArray?) = secureStore.put(key, value)

        override fun notify(n: FfiNotification) {
            main.post { notifications.post(n) }
        }

        override fun labWrite(runId: String, line: String, summary: Boolean) {
            try {
                if (summary) labStore.appendSummary(runId, line) else labStore.append(runId, line)
            } catch (e: Exception) {
                log("lab write $runId failed: $e")
            }
        }

        override fun labDelete(runId: String) {
            runCatching { labStore.delete(runId) }.onFailure { log("lab delete $runId failed: $it") }
        }

        override fun renderPrompts(reqs: List<FfiPromptRequest>) = prompts.render(reqs)

        override fun diagWrite(sessionId: String, seg: Int, lines: List<String>) =
            diagWriter.write(sessionId, seg, lines)

        override fun log(level: FfiLogLevel, msg: String) {
            val p = when (level) {
                FfiLogLevel.DEBUG -> Log.DEBUG
                FfiLogLevel.INFO -> Log.INFO
                FfiLogLevel.WARN -> Log.WARN
                FfiLogLevel.ERROR -> Log.ERROR
            }
            Log.println(p, "MeshCore", msg)
        }
    }

    private fun audioCallbacks() = object : PlatformAudio.Callbacks {
        override fun audioIo(state: FfiAudioIoState) {
            note(DiagNote.audioIo(ioName(state)))
            engine.onAudioIo(state)
            when (state) {
                FfiAudioIoState.CaptureLost -> {
                    captureLost = true
                    main.post { notifications.micPaused() }
                }
                FfiAudioIoState.Restored, FfiAudioIoState.Running -> if (captureLost) {
                    captureLost = false
                    main.post { notifications.cancelAlerts() }
                }
                else -> Unit
            }
            refreshNotification()
        }

        override fun audioRoute(route: FfiAudioRoute) = engine.onAudioRoute(route)

        override fun onCall(active: Boolean) {
            onCall = active
            worker.post { sampleNow() }
        }

        override fun log(msg: String) = this@EngineHost.log(msg)

        override fun diag(note: DiagNote) = this@EngineHost.note(note)
    }

    // MARK: sampling (on_platform, MuteWatchdog, link metrics)

    private var tick = 0L

    private fun scheduleSampling() {
        worker.removeCallbacksAndMessages(SAMPLING)
        worker.postAtTime(object : Runnable {
            override fun run() {
                if (!running) return
                tick++
                audioHandler.post { audio.tick() }
                if (tick % (config.sampleIntervalMs / WATCHDOG_MS).coerceAtLeast(1) == 0L) sampleNow()
                if (tick % (METRICS_MS / WATCHDOG_MS) == 0L) radio.pollMetrics()
                worker.postAtTime(this, SAMPLING, android.os.SystemClock.uptimeMillis() + WATCHDOG_MS)
            }
        }, SAMPLING, android.os.SystemClock.uptimeMillis())
    }

    private fun sampleNow() {
        val d = device.read()
        diagPlatformChanges(d.thermal?.name?.lowercase(), d.battery)
        val sample = FfiPlatformSample(
            wallMs = System.currentTimeMillis().toULong(),
            gps = location.fix(),
            battery = d.battery,
            charging = d.charging,
            thermal = d.thermal,
            background = !processForeground(),
            locked = d.locked,
            onCall = onCall,
            // Android's keep-alive anchor is the FGS (§12.1): session = microphone type.
            anchorSession = FgsTypes.has(fgsActive, android.content.pm.ServiceInfo.FOREGROUND_SERVICE_TYPE_MICROPHONE),
            anchorLocation = FgsTypes.has(fgsActive, android.content.pm.ServiceInfo.FOREGROUND_SERVICE_TYPE_LOCATION),
            anchorLiveActivity = null,
            ble = radioState.ble,
            aware = awareAvail(),
        )
        engine.onPlatformSample(sample)
        aware.setEnabled(sample.aware == FfiRadioAvail.ON)
    }

    // MARK: diagnostics (ridelog/1, contract C6 1.x `diag*`)

    /** A platform note (src `plat`), stamped with the engine clock. */
    override fun note(n: DiagNote) {
        runCatching { engine.diagNote("plat", n.ev, n.fieldsJson) }
    }

    /** A UI note forwarded by the plugin (src `ui`); false when the runtime refused it. */
    fun diagUiNote(ev: String, fieldsJson: String): Boolean = engine.diagNote("ui", ev, fieldsJson)

    /** A user marker from a fixed chip label (`crackle`, `cantHear`, …). */
    fun diagMark(label: String): Boolean = engine.diagMark(label)

    /** Wi-Fi Aware beta (C6 1.2 `awareStatus`): device support and the user's toggle. */
    fun awareStatus(): Pair<Boolean, Boolean> = AwareSupport.supported(app) to AwareSupport.enabled(app)

    /** Turns the Wi-Fi Aware beta on/off (persisted); applies to the running session. */
    fun awareSetEnabled(on: Boolean) {
        AwareSupport.save(app, on && AwareSupport.supported(app))
        aware.setEnabled(awareAvail() == FfiRadioAvail.ON)
        worker.post { sampleNow() }
    }

    /** Recording on/off (persisted; the RAM ring always records). */
    fun diagSetEnabled(on: Boolean) {
        DiagPreference.save(app, on)
        engine.diagConfigure(on, DiagPreference.isDebuggable(app))
    }

    data class DiagStatus(
        val enabled: Boolean,
        val sessionId: String?,
        val rideKey: String?,
        val node: Int?,
        val bytes: Long,
        val sessions: Int,
        val oldestMs: Long?,
    )

    fun diagStatus(): DiagStatus {
        val s = engine.diagStatus()
        diagWriter.drain()
        val u = diagWriter.store?.usage()
        return DiagStatus(s.enabled, s.sessionId, s.rideKey, s.node?.toInt(), u?.bytes ?: 0, u?.sessions ?: 0, u?.oldestMs)
    }

    /** Stored sessions, newest first. */
    fun diagList(): List<com.convlink.voice.engine.diag.DiagSession> {
        engine.diagFlush()
        diagWriter.drain()
        return diagWriter.store?.sessions().orEmpty()
    }

    /** Deletes every diagnostics file (keeps the toggle). */
    fun diagClear() {
        engine.diagFlush()
        diagWriter.drain()
        diagWriter.store?.clear()
    }

    /**
     * Packs the last session (or all) plus `device.json` into a zip under [outDir] (the
     * bridge's FileProvider cache), ready for the share sheet. Without files, the RAM ring
     * is exported.
     */
    fun diagExport(scope: DiagFileStore.Scope, outDir: java.io.File, extraDevice: Map<String, Any?> = emptyMap()): java.io.File {
        engine.diagFlush()
        diagWriter.drain()
        val s = engine.diagStatus()
        val i = info()
        val device = JSONObject()
            .put("platform", "android")
            .put("model", "${Build.MANUFACTURER} ${Build.MODEL}")
            .put("os", "Android ${Build.VERSION.RELEASE} (API ${Build.VERSION.SDK_INT})")
            .put("app", i.appVersion)
            .put("apiVersion", i.apiVersion)
            .put("protocolVersion", i.protocolVersion)
            .put("rtAbiVersion", i.rtAbiVersion)
            .put("engine", i.engine)
            .put("status", i.status)
            .put("audioStrategy", i.audioStrategy)
            .put("rtLinked", i.rtLinked)
            .put("build", if (DiagPreference.isDebuggable(app)) "debug" else "release")
            .put("recording", s.enabled)
        for ((k, v) in extraDevice) device.put(k, v)
        val stamp = java.text.SimpleDateFormat("yyyyMMdd-HHmm", java.util.Locale.ROOT)
            .apply { timeZone = java.util.TimeZone.getTimeZone("UTC") }
            .format(java.util.Date())
        val label = "${s.rideKey ?: "nokey"}-n${s.node?.toString() ?: "x"}-$stamp"
        val store = diagWriter.store ?: DiagFileStore(java.io.File(outDir, "empty"))
        return store.exportZip(scope, label, device.toString(), engine.diagSnapshotRing(), outDir).first
    }

    /** The process is going away: closes the open segment. */
    fun diagTerminate() {
        engine.diagTerminate()
        diagWriter.drain()
    }

    @Volatile
    private var lastForeground: Boolean? = null

    @Volatile
    private var lastPower: String? = null

    /** App visibility and power/Doze changes (notes only on change, from the 1 Hz sample). */
    private fun diagPlatformChanges(thermal: String?, battery: Double?) {
        val fg = processForeground()
        if (fg != lastForeground) {
            lastForeground = fg
            note(DiagNote.app(fg, null))
        }
        val pm = app.getSystemService(android.os.PowerManager::class.java) ?: return
        val doze = pm.isDeviceIdleMode
        val saver = pm.isPowerSaveMode
        val exempt = pm.isIgnoringBatteryOptimizations(app.packageName)
        val key = "$doze|$saver|$exempt|$thermal"
        if (key != lastPower) {
            lastPower = key
            note(DiagNote.power(doze, saver, exempt, thermal, battery))
        }
    }

    private fun ioName(s: FfiAudioIoState): String = when (s) {
        is FfiAudioIoState.Interrupted -> "interrupted:${s.reason.name.lowercase()}"
        else -> s.javaClass.simpleName.replaceFirstChar { it.lowercase() }
    }

    // MARK: helpers

    private fun log(msg: String) {
        Log.i(TAG, msg)
    }

    /** Runs [block] on the main thread and waits (≤ 2 s); direct when already on it. */
    private fun <T> runOnMainSync(block: () -> T): T? {
        if (Looper.myLooper() == Looper.getMainLooper()) return block()
        val latch = java.util.concurrent.CountDownLatch(1)
        var out: T? = null
        main.post {
            try {
                out = block()
            } finally {
                latch.countDown()
            }
        }
        latch.await(MAIN_WAIT_MS, java.util.concurrent.TimeUnit.MILLISECONDS)
        return out
    }

    private fun runOnMainUnit(block: () -> Unit) {
        runOnMainSync(block)
    }

    companion object {
        private const val TAG = "EngineHost"

        /** UI contract major this bridge speaks (mesh-api `API_VERSION` 1.x, §3.7). */
        const val API_MAJOR = 1
        private const val WATCHDOG_MS = 250L
        private const val METRICS_MS = 2_000L
        private const val MAIN_WAIT_MS = 2_000L
        private val SAMPLING = Any()

        // Holds only the application context (no activity), for the process lifetime.
        @SuppressLint("StaticFieldLeak")
        @Volatile
        private var instance: EngineHost? = null

        @Volatile
        private var pendingConfig = EngineHostConfig()

        /** Sets the configuration used when the host is created (call before [obtain]). */
        fun configure(config: EngineHostConfig) {
            pendingConfig = config
        }

        /** The process-wide host (created on first use; cheap, the engine thread is lazy). */
        fun obtain(context: Context): EngineHost = instance ?: synchronized(this) {
            instance ?: EngineHost(context, pendingConfig).also { instance = it }
        }

        fun peek(): EngineHost? = instance

        /** Process importance ≤ FOREGROUND (an activity is visible and resumed). */
        fun processForeground(): Boolean {
            val info = ActivityManager.RunningAppProcessInfo()
            ActivityManager.getMyMemoryState(info)
            return info.importance <= ActivityManager.RunningAppProcessInfo.IMPORTANCE_FOREGROUND
        }
    }
}
