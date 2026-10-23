package com.convlink.voice.engine

import android.Manifest
import android.app.Service
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.content.pm.ServiceInfo
import android.os.Binder
import android.os.IBinder
import android.os.PowerManager
import android.util.Log
import androidx.core.app.ServiceCompat
import androidx.core.content.ContextCompat
import com.convlink.voice.engine.diag.DiagNote
import com.convlink.voice.engine.notify.IntercomAction
import com.convlink.voice.engine.notify.IntercomNotifications

/**
 * Foreground-service types computed from the **current** grants (DESIGN §10.2.3): a
 * location-typed `startForeground` without a granted location permission throws on
 * Android 14+, and a microphone type needs RECORD_AUDIO, so the set is recomputed on every
 * start. Pure (JVM-tested).
 */
object FgsTypes {
    enum class Stage {
        /** Lobby tap: radios only. */
        LOBBY,

        /** Go live / Resume / Lab audio / audio check: capture. */
        LIVE,

        /** Lab render-only or radio-only runs: no capture. */
        LAB_NO_MIC,
    }

    data class Grants(val microphone: Boolean, val fineLocation: Boolean)

    /**
     * LOBBY → CONNECTED_DEVICE; LIVE → + MICROPHONE when granted; LOCATION only when fine
     * location is granted **and** the session shares it ("Share my distance", Lab GPS).
     */
    fun compute(stage: Stage, grants: Grants, shareLocation: Boolean): Int {
        var types = ServiceInfo.FOREGROUND_SERVICE_TYPE_CONNECTED_DEVICE
        if (stage == Stage.LIVE && grants.microphone) types = types or ServiceInfo.FOREGROUND_SERVICE_TYPE_MICROPHONE
        if (stage != Stage.LOBBY && shareLocation && grants.fineLocation) {
            types = types or ServiceInfo.FOREGROUND_SERVICE_TYPE_LOCATION
        }
        return types
    }

    fun grants(context: Context) = Grants(
        microphone = granted(context, Manifest.permission.RECORD_AUDIO),
        fineLocation = granted(context, Manifest.permission.ACCESS_FINE_LOCATION),
    )

    private fun granted(c: Context, p: String) = ContextCompat.checkSelfPermission(c, p) == PackageManager.PERMISSION_GRANTED

    fun has(types: Int, type: Int) = types and type != 0
}

/**
 * The FGS host (DESIGN §10.2.1, §10.2.3): `startForeground` with the types the bridge
 * computed inside the visible tap, `START_NOT_STICKY` (a mic FGS restarted from the
 * background would throw; after process death the next launch offers Resume ride), a
 * `PARTIAL_WAKE_LOCK` for the session, and the ongoing notification whose actions (Mute,
 * Talk latch, Leave, Resume) arrive here. The bridge binds to it for the [EngineHost].
 */
class MeshEngineService : Service() {
    /** Binder handed to the app-local bridge (WP-19). */
    inner class LocalBinder : Binder() {
        val host: EngineHost get() = EngineHost.obtain(applicationContext)
    }

    private val binder = LocalBinder()
    private var wakeLock: PowerManager.WakeLock? = null

    override fun onBind(intent: Intent?): IBinder = binder

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        val host = EngineHost.obtain(applicationContext)
        when (intent?.action) {
            ACTION_FOREGROUND -> {
                val types = intent.getIntExtra(EXTRA_TYPES, ServiceInfo.FOREGROUND_SERVICE_TYPE_CONNECTED_DEVICE)
                if (!enterForeground(host, types)) {
                    stopSelf()
                    return START_NOT_STICKY
                }
            }
            ACTION_STOP -> {
                leaveForeground()
                stopSelf()
            }
            else -> {
                val action = IntercomAction.fromIntent(intent?.action)
                if (action == IntercomAction.RESUME) {
                    // getForegroundService: startForeground must follow; the notification tap
                    // is a while-in-use exemption for the microphone type.
                    enterForeground(host, host.fgsTypes(FgsTypes.Stage.LIVE))
                }
                if (action != null) host.onNotificationAction(action)
            }
        }
        return START_NOT_STICKY
    }

    private fun enterForeground(host: EngineHost, types: Int): Boolean {
        val notification = host.notifications.ongoing(host.intercomState)
        try {
            ServiceCompat.startForeground(this, IntercomNotifications.ID_ONGOING, notification, types)
        } catch (e: RuntimeException) {
            host.note(DiagNote.fgs("fail", types, e.javaClass.simpleName))
            // ForegroundServiceStartNotAllowedException / SecurityException (missing grant or
            // not visible): retry with the radios-only type instead of crashing.
            Log.w(TAG, "startForeground($types) refused: $e")
            val fallback = ServiceInfo.FOREGROUND_SERVICE_TYPE_CONNECTED_DEVICE
            if (types == fallback) return false
            try {
                ServiceCompat.startForeground(this, IntercomNotifications.ID_ONGOING, notification, fallback)
            } catch (e2: RuntimeException) {
                host.note(DiagNote.fgs("fail", fallback, e2.javaClass.simpleName))
                Log.w(TAG, "startForeground(connectedDevice) refused: $e2")
                return false
            }
            host.note(DiagNote.fgs("started", fallback, "fallback"))
            host.foregroundTypesChanged(fallback)
            acquireWakeLock()
            return true
        }
        host.note(DiagNote.fgs("started", types))
        host.foregroundTypesChanged(types)
        acquireWakeLock()
        return true
    }

    private fun leaveForeground() {
        EngineHost.peek()?.note(DiagNote.fgs("stop", 0))
        ServiceCompat.stopForeground(this, ServiceCompat.STOP_FOREGROUND_REMOVE)
        releaseWakeLock()
        EngineHost.peek()?.foregroundTypesChanged(0)
    }

    private fun acquireWakeLock() {
        if (wakeLock?.isHeld == true) return
        val pm = getSystemService(PowerManager::class.java)
        wakeLock = pm.newWakeLock(PowerManager.PARTIAL_WAKE_LOCK, WAKE_TAG).apply {
            setReferenceCounted(false)
            acquire(WAKE_TIMEOUT_MS)
        }
    }

    private fun releaseWakeLock() {
        wakeLock?.let { if (it.isHeld) it.release() }
        wakeLock = null
    }

    override fun onDestroy() {
        releaseWakeLock()
        EngineHost.peek()?.foregroundTypesChanged(0)
        super.onDestroy()
    }

    companion object {
        private const val TAG = "MeshEngineService"
        const val ACTION_FOREGROUND = "com.convlink.voice.engine.action.FOREGROUND"
        const val ACTION_STOP = "com.convlink.voice.engine.action.STOP"
        const val EXTRA_TYPES = "types"
        private const val WAKE_TAG = "convlinkvoice:meshengine"

        /** Upper bound of one ride; renewed on every foreground start. */
        private const val WAKE_TIMEOUT_MS = 12 * 60 * 60 * 1000L

        /**
         * `startForegroundService` with [types]: call it **inside the visible tap** (lobby,
         * Go live), `startForeground` follows in onStartCommand within 5 s.
         */
        fun startForeground(context: Context, types: Int) {
            val i = Intent(context, MeshEngineService::class.java)
                .setAction(ACTION_FOREGROUND)
                .putExtra(EXTRA_TYPES, types)
            ContextCompat.startForegroundService(context, i)
        }

        fun stop(context: Context) {
            try {
                context.startService(Intent(context, MeshEngineService::class.java).setAction(ACTION_STOP))
            } catch (e: IllegalStateException) {
                // Not running (background start refused): nothing to stop.
                Log.i(TAG, "stop: service not running ($e)")
            }
        }
    }
}
