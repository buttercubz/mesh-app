package com.convlink.voice.engine.audio

import android.util.Log

/** RT statistics of the core's audio plane (`MeshRtStats`, contract C5). */
data class RtStats(
    val capturedFrames: Long,
    val renderedFrames: Long,
    val captureOverruns: Long,
    val renderUnderruns: Long,
    val captureCallbacks: Long,
    val renderCallbacks: Long,
    val captureRmsDbfs: Float,
    val renderRmsDbfs: Float,
    val generation: Long,
)

/** One audio glitch report: an Oboe/AAudio xrun on the input or output stream. */
fun interface XrunListener {
    /**
     * [input] true for the capture stream (`mesh_rt_report` code 4), false for render
     * (code 3); [newXruns] xruns since the previous report (≥ 1); [total] running count.
     */
    fun onXrun(input: Boolean, newXruns: Long, total: Long)
}

/**
 * Turns the RT plane's xrun signals into [XrunListener] calls (pure, JVM-testable). Two
 * sources feed it: native per-stream xrun counters ([onCounts], polled by
 * [OboeBridge.pollXruns] when the shim exports them) and the core's RT report log line
 * (`mesh-ffi: RT report #N code C`, [onReportLog]) for shims without counters. Each
 * source is de-duplicated on its own; use one per build.
 */
class XrunTracker {
    private val listeners = java.util.concurrent.CopyOnWriteArrayList<XrunListener>()
    private var lastCounts = longArrayOf(0, 0) // [output, input]
    private var lastReport = 0L
    private val totals = longArrayOf(0, 0)

    fun add(l: XrunListener) {
        listeners.addIfAbsent(l)
    }

    fun remove(l: XrunListener) {
        listeners.remove(l)
    }

    /** Native cumulative counters; a counter that went down (stream reopened) restarts. */
    @Synchronized
    fun onCounts(output: Long, input: Long) {
        emitDelta(false, output)
        emitDelta(true, input)
    }

    private fun emitDelta(input: Boolean, value: Long) {
        val i = if (input) 1 else 0
        val prev = lastCounts[i]
        val delta = if (value < prev) value else value - prev
        lastCounts[i] = value
        if (delta > 0) emit(input, delta)
    }

    /**
     * A core log line; returns true when it was an xrun report. Reports between two log
     * lines are coalesced by the core (count advances, only the last code is kept): the
     * whole count difference is attributed to that code's stream.
     */
    @Synchronized
    fun onReportLog(message: String): Boolean {
        val m = REPORT.find(message) ?: return false
        val count = m.groupValues[1].toLongOrNull() ?: return false
        val code = m.groupValues[2].toIntOrNull() ?: return false
        val delta = if (count < lastReport) count else count - lastReport
        lastReport = count
        val input = when (code) {
            CODE_OUTPUT_XRUN -> false
            CODE_INPUT_XRUN -> true
            else -> return false
        }
        if (delta > 0) emit(input, delta)
        return true
    }

    private fun emit(input: Boolean, delta: Long) {
        val i = if (input) 1 else 0
        totals[i] += delta
        val total = totals[i]
        listeners.forEach { l ->
            // A failing subscriber must not break the audio bookkeeping of the others.
            runCatching { l.onXrun(input, delta, total) }
        }
    }

    companion object {
        /** `mesh_rt_report` codes written by meshaudio.cpp. */
        const val CODE_OUTPUT_XRUN = 3
        const val CODE_INPUT_XRUN = 4
        private val REPORT = Regex("""RT report #(\d+) code (\d+)""")
    }
}

/** The Oboe stream pair (abstracted so the session logic is JVM-testable). */
interface AudioIo {
    /** Opens and starts the streams; returns 0 or a negative Oboe result. Capture starts muted. */
    fun open(capture: Boolean, render: Boolean, inputDeviceId: Int, outputDeviceId: Int, bypassVp: Boolean): Int
    fun close()

    /** Connects (true) or mutes (false) the capture sink (`StartCapture` / `StopCapture`). */
    fun setCaptureEnabled(enabled: Boolean)
    fun stats(): RtStats?
    fun setFlags(flags: Int)

    /** Called with (isInput, oboeResult) after a stream was closed by an error. */
    var onStreamError: ((Boolean, Int) -> Unit)?
}

/**
 * JNI bridge to `libmeshaudio.so` (src/main/cpp/meshaudio.cpp): Oboe streams feeding
 * `mesh_rt_capture` / `mesh_rt_render`, resolved with dlsym (tone fallback when absent).
 */
object OboeBridge : AudioIo {
    private const val TAG = "OboeBridge"

    /** False on the JVM (unit tests) or when the shim failed to load. */
    val loaded: Boolean by lazy {
        try {
            System.loadLibrary("meshaudio")
            true
        } catch (e: UnsatisfiedLinkError) {
            Log.w(TAG, "libmeshaudio not loaded: ${e.message}")
            false
        }
    }

    @Volatile
    override var onStreamError: ((Boolean, Int) -> Unit)? = null

    override fun open(capture: Boolean, render: Boolean, inputDeviceId: Int, outputDeviceId: Int, bypassVp: Boolean): Int =
        if (loaded) nativeOpen(capture, render, inputDeviceId, outputDeviceId, bypassVp) else ERROR_UNAVAILABLE

    override fun close() {
        if (loaded) nativeClose()
    }

    override fun setCaptureEnabled(enabled: Boolean) {
        if (loaded) nativeSetCaptureEnabled(enabled)
    }

    override fun setFlags(flags: Int) {
        if (loaded) nativeSetFlags(flags)
    }

    /** True when the shim resolved every `mesh_rt_*` symbol (otherwise it plays a tone). */
    fun hasRt(): Boolean = loaded && nativeHasRt()

    override fun stats(): RtStats? {
        if (!loaded) return null
        val v = LongArray(STATS_LEN)
        if (!nativeStats(v)) return null
        return RtStats(
            capturedFrames = v[0], renderedFrames = v[1], captureOverruns = v[2], renderUnderruns = v[3],
            captureCallbacks = v[4], renderCallbacks = v[5], captureRmsDbfs = v[6] / 100f,
            renderRmsDbfs = v[7] / 100f, generation = v[8],
        )
    }

    /** Xrun reports (codes 3/4) for the diagnostics recorder: see [addXrunListener]. */
    val xruns = XrunTracker()

    /** Subscribes to Oboe xrun reports (diagnostics lane hook). */
    fun addXrunListener(l: XrunListener) = xruns.add(l)

    fun removeXrunListener(l: XrunListener) = xruns.remove(l)

    /**
     * Feeds a core log message (`PlatformHooks.log`) so `mesh-ffi: RT report #N code C`
     * lines reach the xrun listeners; returns true when the line was an xrun report.
     */
    fun noteRtLog(message: String): Boolean = xruns.onReportLog(message)

    @Volatile
    private var countersAvailable = true

    /**
     * Polls the shim's cumulative per-stream xrun counters (≈ 1 Hz, off the audio thread)
     * when it exports `nativeXrunCounts`; returns false when it does not (then the log path
     * [noteRtLog] is the source).
     */
    fun pollXruns(): Boolean {
        if (!loaded || !countersAvailable) return false
        val v = LongArray(2)
        val ok = try {
            nativeXrunCounts(v)
        } catch (_: UnsatisfiedLinkError) {
            // Older shim without the counters export: fall back to the RT report log line.
            countersAvailable = false
            false
        }
        if (ok) xruns.onCounts(v[0], v[1])
        return ok
    }

    /** Called from the Oboe error thread (meshaudio.cpp `onErrorAfterClose`). */
    @JvmStatic
    fun onStreamError(input: Boolean, code: Int) {
        onStreamError?.invoke(input, code)
    }

    /** `oboe::Result::ErrorUnavailable`. */
    const val ERROR_UNAVAILABLE = -889
    private const val STATS_LEN = 9

    @JvmStatic
    private external fun nativeOpen(capture: Boolean, render: Boolean, inDev: Int, outDev: Int, bypassVp: Boolean): Int

    @JvmStatic
    private external fun nativeClose()

    @JvmStatic
    private external fun nativeSetCaptureEnabled(enabled: Boolean)

    @JvmStatic
    private external fun nativeHasRt(): Boolean

    @JvmStatic
    private external fun nativeSetFlags(flags: Int)

    @JvmStatic
    private external fun nativeStats(out: LongArray): Boolean

    /** Optional export (change request to meshaudio.cpp): `[output, input]` cumulative xruns. */
    @JvmStatic
    private external fun nativeXrunCounts(out: LongArray): Boolean
}
