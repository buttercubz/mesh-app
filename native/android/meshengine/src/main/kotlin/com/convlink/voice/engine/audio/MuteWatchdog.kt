package com.convlink.voice.engine.audio

/**
 * Detects a silent capture stop (DESIGN §10.1.4, used exactly as on iOS): if the capture
 * callback counter of `mesh_rt_stats` has not advanced for more than [stallMs] while
 * capture should be running, it reports a stall once, then re-arms when the counter moves
 * again. Pure; the caller polls [check].
 */
class MuteWatchdog(val stallMs: Long = DEFAULT_STALL_MS) {
    private var lastCount: Long? = null
    private var lastChange = 0L
    private var fired = false

    /** Starts watching from [now] (streams just opened or reopened). */
    fun arm(count: Long, now: Long) {
        lastCount = count
        lastChange = now
        fired = false
    }

    /** Stops watching (streams closed, capture intentionally off). */
    fun disarm() {
        lastCount = null
        fired = false
    }

    val isArmed: Boolean get() = lastCount != null

    /** Feeds the current callback counter; returns true exactly once per stall. */
    fun check(count: Long, now: Long): Boolean {
        val last = lastCount ?: return false
        if (count != last) {
            lastCount = count
            lastChange = now
            fired = false
            return false
        }
        if (fired || now - lastChange <= stallMs) return false
        fired = true
        return true
    }

    companion object {
        const val DEFAULT_STALL_MS = 1_500L
    }
}
