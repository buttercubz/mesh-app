package com.convlink.voice.plugin

import android.app.PendingIntent
import android.content.Intent
import android.os.SystemClock
import android.util.Log
import android.view.WindowManager
import androidx.lifecycle.Lifecycle
import com.convlink.voice.engine.EngineHost
import com.convlink.voice.engine.EngineHostConfig
import com.convlink.voice.engine.EngineHostException
import com.convlink.voice.engine.audio.GestureCommand
import com.convlink.voice.engine.diag.DiagFileStore
import com.convlink.voice.engine.diag.DiagNote
import com.convlink.voice.engine.notify.NotificationCatalog
import com.getcapacitor.JSObject
import com.getcapacitor.PluginCall
import com.getcapacitor.PluginMethod
import com.getcapacitor.annotation.CapacitorPlugin
import com.getcapacitor.annotation.Permission

/**
 * App-local MeshIntercom bridge (contract C6, DESIGN §2.4, §11.1; WP-19, Android half).
 *
 * Forwards the `@meshapp/capacitor-intercom` API to :meshengine's [EngineHost]:
 * - `send` is a JSON pass-through of mesh-api `UiCommand` → `CommandResult` and rejects
 *   the ★ gesture-gated commands with `useGestureCommand`;
 * - `gestureCommand` (and `goLive` / `resumeMic`) rejects with `notForeground` unless the
 *   activity is resumed, then lets the host do the OS work synchronously
 *   (`startForegroundService` with microphone [+ location], Oboe streams muted) before the
 *   engine sees the command;
 * - `ptt` writes the RT flag first (fast path, T8), then sends `{type:"ptt"}`;
 * - UI events reach JS as `notifyListeners("event", {json})`.
 *
 * Rejections carry the mesh-api `ErrorCode` as both message and code. Capacitor calls the
 * plugin methods on its plugin thread (never the main thread), so the host's gesture path
 * may run there directly; UI work hops to the main thread.
 */
@CapacitorPlugin(
    name = "MeshIntercom",
    permissions = [
        Permission(alias = PermissionCenter.MICROPHONE, strings = [android.Manifest.permission.RECORD_AUDIO]),
        Permission(
            alias = PermissionCenter.BLUETOOTH,
            strings = [
                android.Manifest.permission.BLUETOOTH_SCAN,
                android.Manifest.permission.BLUETOOTH_ADVERTISE,
                android.Manifest.permission.BLUETOOTH_CONNECT,
            ],
        ),
        Permission(alias = PermissionCenter.NOTIFICATIONS, strings = [android.Manifest.permission.POST_NOTIFICATIONS]),
        Permission(
            alias = PermissionCenter.LOCATION,
            strings = [
                android.Manifest.permission.ACCESS_FINE_LOCATION,
                android.Manifest.permission.ACCESS_COARSE_LOCATION,
            ],
        ),
        Permission(alias = PermissionCenter.NEARBY_WIFI, strings = [android.Manifest.permission.NEARBY_WIFI_DEVICES]),
    ],
)
class MeshIntercomPlugin : PlatformPlugin() {
    /** The engine host, or null when it could not be created ([hostFailure] says why). */
    @Volatile
    internal var host: EngineHost? = null
    private var hostFailure: String? = null
    private var engineApiVersion: String? = null

    /** The activity is resumed: the only state in which gesture commands are accepted. */
    @Volatile
    private var resumed = false

    /** Set by an accepted goLive, cleared by stopEngine (EngineHost does not expose it). */
    @Volatile
    private var live = false

    override fun load() {
        super.load()
        resumed = activity.lifecycle.currentState.isAtLeast(Lifecycle.State.RESUMED)
        try {
            EngineHost.configure(EngineHostConfig(appLabel = appLabel()))
            val h = EngineHost.obtain(context)
            h.foregroundCheck = { resumed }
            h.setEventListener { json -> notifyListeners(EVENT, JSObject().put("json", json)) }
            h.setNotificationContentIntent(contentIntent())
            h.setUiAttached(resumed)
            host = h
        } catch (e: EngineHostException) {
            hostFailure = e.code
            if (e.code == "apiVersionMismatch") engineApiVersion = e.message?.substringAfter("engine API ")
            Log.e(TAG, "engine host unavailable: ${e.code} ${e.message}")
        } catch (e: Throwable) {
            // UnsatisfiedLinkError / JNA load failure: run as platform-only (getInfo says so).
            hostFailure = "internal"
            Log.e(TAG, "engine host unavailable", e)
        }
    }

    // MARK: lifecycle (Capacitor forwards the activity callbacks on the main thread)

    override fun handleOnResume() {
        super.handleOnResume()
        resumed = true
        val h = host ?: return
        h.setUiAttached(true)
        if (isRunning(h)) runCatching { h.send("""{"type":"requestSnapshot"}""") }
    }

    override fun handleOnPause() {
        resumed = false
        host?.setUiAttached(false)
        // setKeepScreenOn is reset on every background transition (C6).
        activity.window.clearFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
        super.handleOnPause()
    }

    // MARK: info

    @PluginMethod
    fun getInfo(call: PluginCall) {
        val h = host
        val out = JSObject().put("platform", "android").put("appVersion", appVersionName())
        if (h == null) {
            val api = engineApiVersion?.toIntOrNull()?.let { "${it shr 8}.${it and 0xFF}" } ?: API_VERSION
            call.resolve(out.put("engine", "platform-only").put("apiVersion", api).put("protocolVersion", PROTOCOL_VERSION))
            return
        }
        val info = h.info()
        call.resolve(
            out.put("engine", info.engine).put("apiVersion", info.apiVersion)
                .put("protocolVersion", info.protocolVersion),
        )
    }

    @PluginMethod
    fun getStatus(call: PluginCall) {
        val h = host
        if (h == null) {
            call.resolve(JSObject().put("engine", "idle").put("live", false).put("uiAttached", false))
            return
        }
        val info = h.info()
        val running = info.status == "running" || info.status == "audioCheck"
        call.resolve(
            JSObject().put("engine", if (running) "running" else "idle")
                .put("live", running && live).put("uiAttached", info.uiAttached),
        )
    }

    // MARK: engine lifecycle

    /** Gesture required: radios + engine thread (FGS connectedDevice). Audio stays off. */
    @PluginMethod
    fun startEngine(call: PluginCall) {
        if (!resumed) return reject(call, "notForeground")
        val start = BridgeSupport.sessionStart(call.data) ?: return reject(call, "invalidParams")
        val h = requireHost(call) ?: return
        try {
            h.start(start)
            call.resolve()
        } catch (e: Exception) {
            reject(call, BridgeSupport.errorCode(e), e.message)
        }
    }

    /** Ordered shutdown: the host keeps one radio stack for the process (WP-15/16 xreq). */
    @PluginMethod
    fun stopEngine(call: PluginCall) {
        val h = requireHost(call) ?: return
        h.stop()
        live = false
        call.resolve()
    }

    // MARK: commands

    @PluginMethod
    fun send(call: PluginCall) {
        val json = call.getString("command") ?: return reject(call, "invalidParams")
        if (GestureCommand.requiresGesture(json)) return reject(call, "useGestureCommand")
        val h = requireHost(call) ?: return
        val t0 = SystemClock.elapsedRealtime()
        val result = h.send(json)
        noteCommand(h, json, result, false, t0)
        call.resolve(JSObject().put("result", result))
    }

    @PluginMethod
    fun gestureCommand(call: PluginCall) {
        val json = call.getString("command") ?: return reject(call, "invalidParams")
        runGesture(call, json) { call.resolve(JSObject().put("result", it)) }
    }

    @PluginMethod
    fun goLive(call: PluginCall) = runGesture(call, """{"type":"goLive"}""") { resolveSugar(call, it) }

    @PluginMethod
    fun resumeMic(call: PluginCall) = runGesture(call, """{"type":"resumeMic"}""") { resolveSugar(call, it) }

    /** PTT fast path: RT flag first (T8), then `{type:"ptt"}`. */
    @PluginMethod
    fun ptt(call: PluginCall) {
        val receivedMs = SystemClock.elapsedRealtimeNanos() / 1e6
        val h = requireHost(call) ?: return
        val down = call.getBoolean("down") ?: return reject(call, "invalidParams")
        val tUiMs = call.getDouble("tUiMs")
        if (tUiMs != null && !tUiMs.isFinite()) return reject(call, "invalidParams")
        if (!isRunning(h)) return reject(call, "engineNotRunning")
        h.ptt(down)
        val doneMs = SystemClock.elapsedRealtimeNanos() / 1e6
        Log.i(T8_TAG, "ptt down=$down tUiMs=${tUiMs ?: "-"} rxMs=%.1f sentMs=%.1f".format(receivedMs, doneMs))
        call.resolve()
    }

    /** Localised notification texts from the UI catalogs (`notices` + `errors`). */
    @PluginMethod
    fun setNotificationCatalog(call: PluginCall) {
        val h = requireHost(call) ?: return
        val title = call.getString("title")
        val raw = call.getObject("templates")
        if (title == null || raw == null) return reject(call, "invalidParams")
        val templates = buildMap {
            raw.keys().forEach { k -> (raw.opt(k) as? String)?.let { put(k, it) } }
        }
        h.setNotificationCatalog(NotificationCatalog(title, templates))
        call.resolve()
    }

    // MARK: gesture plumbing

    /** Foreground first (synchronously, inside the tap), then the host's gesture path. */
    private fun runGesture(call: PluginCall, json: String, done: (String) -> Unit) {
        if (!resumed) return reject(call, "notForeground")
        val h = requireHost(call) ?: return
        val t0 = SystemClock.elapsedRealtime()
        try {
            val result = h.gestureCommand(json)
            noteCommand(h, json, result, true, t0)
            if (BridgeSupport.isOk(result) && BridgeSupport.commandType(json) == "goLive") live = true
            done(result)
        } catch (e: Exception) {
            reject(call, BridgeSupport.errorCode(e), e.message)
        }
    }

    private fun resolveSugar(call: PluginCall, result: String) {
        if (BridgeSupport.isOk(result)) call.resolve() else reject(call, BridgeSupport.resultCode(result) ?: "internal")
    }

    // MARK: diagnostics (C6 1.1, ridelog/1)

    /**
     * A `cmd` diagnostics note for a command the UI sent: id, gesture, answer code and
     * round trip only (never the parameters, which may hold names or invites).
     */
    private fun noteCommand(h: EngineHost, json: String, result: String, gesture: Boolean, t0: Long) {
        val type = BridgeSupport.commandType(json) ?: return
        if (type == "requestSnapshot") return
        val fields = JSObject()
            .put("name", type)
            .put("gesture", gesture)
            .put("ok", BridgeSupport.isOk(result))
            .put("err", BridgeSupport.resultCode(result) ?: org.json.JSONObject.NULL)
            .put("ms", SystemClock.elapsedRealtime() - t0)
        runCatching { h.diagUiNote("cmd", fields.toString()) }
    }

    /** One `perm` note per permission answered to the UI, with the exact API behind it. */
    override fun onPermissionStatus(status: JSObject, asked: Collection<String>) {
        val h = host ?: return
        for (alias in status.keys()) {
            val via = if (alias in asked) "request" else "check"
            h.note(DiagNote.perm(alias, status.optString(alias), via, "Plugin.getPermissionState($alias)"))
        }
    }

    @PluginMethod
    fun diagStatus(call: PluginCall) {
        val h = requireHost(call) ?: return
        val s = h.diagStatus()
        val nul = org.json.JSONObject.NULL
        call.resolve(
            JSObject().put("enabled", s.enabled).put("sessionId", s.sessionId ?: nul).put("rideKey", s.rideKey ?: nul)
                .put("node", s.node ?: nul).put("bytes", s.bytes).put("sessions", s.sessions)
                .put("oldestMs", s.oldestMs ?: nul),
        )
    }

    @PluginMethod
    fun diagList(call: PluginCall) {
        val h = requireHost(call) ?: return
        val arr = com.getcapacitor.JSArray()
        for (s in h.diagList()) {
            arr.put(
                JSObject().put("sessionId", s.sessionId).put("bytes", s.bytes).put("segments", s.segments.size)
                    .put("modifiedMs", s.modifiedMs),
            )
        }
        call.resolve(JSObject().put("sessions", arr))
    }

    /** Packs the last session (or all) into a zip under `cache/lab/diag` and shares it. */
    @PluginMethod
    fun diagExport(call: PluginCall) {
        val h = requireHost(call) ?: return
        val scope = when (call.getString("scope")) {
            "last" -> DiagFileStore.Scope.LAST
            "all" -> DiagFileStore.Scope.ALL
            else -> return reject(call, "invalidParams", "scope")
        }
        val sessions = h.diagList().size.let { if (scope == DiagFileStore.Scope.LAST) minOf(1, it) else it }
        val zip = try {
            // The FileProvider exposes cache/lab/ (and below) only.
            h.diagExport(scope, java.io.File(context.cacheDir, "lab/diag"))
        } catch (e: Exception) {
            return reject(call, "storageFailed", e.message)
        }
        val uri = androidx.core.content.FileProvider.getUriForFile(context, "${context.packageName}.fileprovider", zip)
        val send = Intent(Intent.ACTION_SEND)
            .setType("application/zip")
            .putExtra(Intent.EXTRA_STREAM, uri)
            .addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
        activity.runOnUiThread {
            try {
                activity.startActivity(Intent.createChooser(send, null))
                call.resolve(JSObject().put("path", zip.absolutePath).put("mime", "application/zip").put("sessions", sessions))
            } catch (e: android.content.ActivityNotFoundException) {
                reject(call, "notSupported", e.message)
            }
        }
    }

    @PluginMethod
    fun diagClear(call: PluginCall) {
        val h = requireHost(call) ?: return
        h.diagClear()
        call.resolve()
    }

    @PluginMethod
    fun diagSetEnabled(call: PluginCall) {
        val h = requireHost(call) ?: return
        val on = call.getBoolean("enabled") ?: return reject(call, "invalidParams", "enabled")
        h.diagSetEnabled(on)
        call.resolve()
    }

    // MARK: Wi-Fi Aware beta (C6 1.2, WP-24)

    /** Device support (FEATURE_WIFI_AWARE) and the user's beta toggle. */
    @PluginMethod
    fun awareStatus(call: PluginCall) {
        val h = requireHost(call) ?: return
        val (supported, enabled) = h.awareStatus()
        call.resolve(JSObject().put("supported", supported).put("enabled", enabled))
    }

    @PluginMethod
    fun awareSetEnabled(call: PluginCall) {
        val h = requireHost(call) ?: return
        val on = call.getBoolean("enabled") ?: return reject(call, "invalidParams", "enabled")
        h.awareSetEnabled(on)
        call.resolve()
    }

    @PluginMethod
    fun diagMark(call: PluginCall) {
        val h = requireHost(call) ?: return
        val label = call.getString("label") ?: return reject(call, "invalidParams", "label")
        if (!h.diagMark(label)) return reject(call, "invalidParams", "label")
        call.resolve()
    }

    // MARK: helpers

    private fun requireHost(call: PluginCall): EngineHost? {
        host?.let { return it }
        reject(call, if (hostFailure == "apiVersionMismatch") "apiVersionMismatch" else "notSupported")
        return null
    }

    private fun isRunning(h: EngineHost): Boolean = h.info().status.let { it == "running" || it == "audioCheck" }

    private fun appVersionName(): String = runCatching {
        context.packageManager.getPackageInfo(context.packageName, 0).versionName
    }.getOrNull() ?: "0.0.0"

    private fun appLabel(): String = runCatching {
        val p = context.packageManager.getPackageInfo(context.packageName, 0)
        "${p.versionName} (${p.longVersionCode})"
    }.getOrDefault("0.0.0 (0)")

    /** Tapping the ongoing notification brings the single activity back. */
    private fun contentIntent(): PendingIntent {
        val intent = Intent(context, activity.javaClass)
            .addFlags(Intent.FLAG_ACTIVITY_SINGLE_TOP or Intent.FLAG_ACTIVITY_REORDER_TO_FRONT)
        return PendingIntent.getActivity(context, 0, intent, PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT)
    }

    companion object {
        private const val TAG = "MeshIntercom"
        private const val T8_TAG = "MeshIntercomT8"
        private const val EVENT = "event"

        /** UI contract version used when the engine is not linked (mesh-api `API_VERSION`). */
        const val API_VERSION = "1.1"

        /** Wire protocol version (Appendix A). */
        const val PROTOCOL_VERSION = 1
    }
}
