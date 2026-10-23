package com.convlink.voice.engine.audio

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class XrunTrackerTest {
    private data class Event(val input: Boolean, val delta: Long, val total: Long)

    @Test
    fun countersEmitDeltasPerStream() {
        val t = XrunTracker()
        val events = mutableListOf<Event>()
        t.add { input, d, total -> events.add(Event(input, d, total)) }
        t.onCounts(0, 0)
        t.onCounts(2, 0)
        t.onCounts(2, 1)
        t.onCounts(3, 1)
        // A reopened stream restarts its counter.
        t.onCounts(1, 1)
        assertEquals(
            listOf(Event(false, 2, 2), Event(true, 1, 1), Event(false, 1, 3), Event(false, 1, 4)),
            events,
        )
    }

    @Test
    fun coreReportLogLinesMapCodes3And4() {
        val t = XrunTracker()
        val events = mutableListOf<Event>()
        t.add { input, d, total -> events.add(Event(input, d, total)) }
        assertTrue(t.onReportLog("mesh-ffi: RT report #1 code 3"))
        assertTrue(t.onReportLog("mesh-ffi: RT report #4 code 4"))
        // Route/stream-closed codes are not xruns; the count still advances.
        assertFalse(t.onReportLog("mesh-ffi: RT report #5 code 1"))
        assertFalse(t.onReportLog("something else"))
        assertTrue(t.onReportLog("mesh-ffi: RT report #6 code 3"))
        assertEquals(listOf(Event(false, 1, 1), Event(true, 3, 3), Event(false, 1, 2)), events)
    }

    @Test
    fun failingListenerDoesNotStopOthers() {
        val t = XrunTracker()
        var seen = 0
        t.add { _, _, _ -> error("boom") }
        val l = XrunListener { _, _, _ -> seen++ }
        t.add(l)
        t.onCounts(1, 0)
        t.remove(l)
        t.onCounts(2, 0)
        assertEquals(1, seen)
    }
}
