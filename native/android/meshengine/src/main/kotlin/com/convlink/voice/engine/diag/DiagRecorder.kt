package com.convlink.voice.engine.diag

import android.content.Context
import android.content.pm.ApplicationInfo
import java.util.concurrent.ExecutorService
import java.util.concurrent.Executors
import java.util.concurrent.TimeUnit

/**
 * `PlatformHooks.diag_write` over [DiagFileStore]. The hook runs on the engine thread, so
 * writes go to a single background thread and the engine never waits for the disk. A
 * store that could not be created (null) leaves only the runtime's RAM ring.
 */
class DiagWriter(val store: DiagFileStore?, private val log: (String) -> Unit) {
    private val executor: ExecutorService = Executors.newSingleThreadExecutor { r ->
        Thread(r, "meshengine-diag").apply { isDaemon = true }
    }

    fun write(sessionId: String, seg: Int, lines: List<String>) {
        val s = store ?: return
        executor.execute {
            try {
                s.append(sessionId, seg, lines)
            } catch (e: Exception) {
                log("diag: append to $sessionId/$seg failed: $e")
            }
        }
    }

    /** Waits (≤ 2 s) until every queued batch is on disk (before an export or listing). */
    fun drain() {
        runCatching { executor.submit {}.get(2, TimeUnit.SECONDS) }
    }
}

/**
 * The recording preference (SharedPreferences: a plain boolean, never a secret). Default:
 * on in debuggable builds and while the app version is below 1.0 (field-testing period).
 */
object DiagPreference {
    private const val PREFS = "convlink.diag"
    private const val KEY = "enabled"

    fun defaultEnabled(appLabel: String, debug: Boolean): Boolean {
        if (debug) return true
        val major = appLabel.substringBefore('.').trim().toIntOrNull() ?: 0
        return major < 1
    }

    fun isDebuggable(context: Context): Boolean =
        context.applicationInfo.flags and ApplicationInfo.FLAG_DEBUGGABLE != 0

    fun load(context: Context, appLabel: String): Boolean {
        val p = context.getSharedPreferences(PREFS, Context.MODE_PRIVATE)
        if (!p.contains(KEY)) return defaultEnabled(appLabel, isDebuggable(context))
        return p.getBoolean(KEY, true)
    }

    fun save(context: Context, on: Boolean) {
        context.getSharedPreferences(PREFS, Context.MODE_PRIVATE).edit().putBoolean(KEY, on).apply()
    }
}
