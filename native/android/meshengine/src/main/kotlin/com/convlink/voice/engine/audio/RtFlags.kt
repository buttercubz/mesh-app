package com.convlink.voice.engine.audio

/**
 * Mirror of the word written with `mesh_rt_set_flags` (contract C5): bit0 PTT down, bit1
 * hard mute, bit2 motorcycle built-in mic mute, bit3 PTT latch toggled (an edge the engine
 * consumes). The RT ABI only sets the whole word, so the platform keeps the current value
 * here; notification actions (Mute, Talk latch) and the bridge's PTT fast path write it.
 */
class RtFlags(private val write: (Int) -> Unit) {
    private var flags = 0

    @get:Synchronized
    val value: Int get() = flags

    @Synchronized
    fun set(bit: Int, on: Boolean): Int {
        flags = if (on) flags or bit else flags and bit.inv()
        write(flags)
        return flags
    }

    fun setPttDown(down: Boolean) = set(PTT_DOWN, down)

    fun setHardMute(muted: Boolean) = set(HARD_MUTE, muted)

    fun toggleHardMute(): Boolean = synchronized(this) {
        val muted = flags and HARD_MUTE == 0
        set(HARD_MUTE, muted)
        muted
    }

    fun setBuiltinMicMute(muted: Boolean) = set(BUILTIN_MIC_MUTE, muted)

    /**
     * Raises the latch-toggle edge (Talk latch). The runtime keeps the edge until the engine
     * consumes it, so the bit is written set and then cleared from the mirror; a later
     * toggle writes the edge again.
     */
    @Synchronized
    fun latchToggle() {
        write(flags or LATCH_TOGGLE)
        flags = flags and LATCH_TOGGLE.inv()
    }

    val hardMuted: Boolean get() = value and HARD_MUTE != 0

    companion object {
        const val PTT_DOWN = 1 shl 0
        const val HARD_MUTE = 1 shl 1
        const val BUILTIN_MIC_MUTE = 1 shl 2
        const val LATCH_TOGGLE = 1 shl 3
    }
}
