package com.convlink.voice.engine.audio

import android.media.AudioManager
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/** Music coexistence (2026-10-23 field test): duck while a peer talks, never pause music. */
class DuckingFocusTest {
    private class FakePort(var grant: Boolean = true) : DuckingFocus.Port {
        var requests = 0
        var abandons = 0

        override fun request(): Boolean {
            requests++
            return grant
        }

        override fun abandon() {
            abandons++
        }
    }

    @Test
    fun requestsOnVoiceAndReleasesAfterHangover() {
        val port = FakePort()
        val f = DuckingFocus(port)
        f.setVoiceActive(true, now = 0)
        assertTrue(f.held)
        assertEquals(1, port.requests)
        f.setVoiceActive(false, now = 1_000)
        f.tick(1_000 + DuckingFocus.RELEASE_HANGOVER_MS - 1)
        assertTrue("music stays ducked between overs", f.held)
        f.tick(1_000 + DuckingFocus.RELEASE_HANGOVER_MS)
        assertFalse(f.held)
        assertEquals(1, port.abandons)
    }

    @Test
    fun consecutiveOversKeepOneGrant() {
        val port = FakePort()
        val f = DuckingFocus(port)
        f.setVoiceActive(true, 0)
        f.setVoiceActive(false, 500)
        f.setVoiceActive(true, 900)
        f.tick(500 + DuckingFocus.RELEASE_HANGOVER_MS)
        assertTrue("voice active again: never released mid-over", f.held)
        assertEquals(1, port.requests)
        assertEquals(0, port.abandons)
    }

    @Test
    fun focusLossForgetsGrantAndNextVoiceReRequests() {
        val port = FakePort()
        val f = DuckingFocus(port)
        f.setVoiceActive(true, 0)
        f.lost() // the music app took focus back
        assertFalse(f.held)
        f.release()
        assertEquals("nothing to abandon after a loss", 0, port.abandons)
        f.setVoiceActive(true, 10)
        assertEquals(2, port.requests)
        assertTrue(f.held)
    }

    @Test
    fun deniedRequestIsRetriedOnNextVoice() {
        val port = FakePort(grant = false)
        val f = DuckingFocus(port)
        f.setVoiceActive(true, 0)
        assertFalse(f.held)
        port.grant = true
        f.setVoiceActive(true, 20)
        assertTrue(f.held)
    }

    @Test
    fun onlyLossesAreReportedAndDuckRequestsAreIgnored() {
        assertEquals(false, DuckingFocus.lossKind(AudioManager.AUDIOFOCUS_LOSS))
        assertEquals(true, DuckingFocus.lossKind(AudioManager.AUDIOFOCUS_LOSS_TRANSIENT))
        assertNull(DuckingFocus.lossKind(AudioManager.AUDIOFOCUS_LOSS_TRANSIENT_CAN_DUCK))
        assertNull(DuckingFocus.lossKind(AudioManager.AUDIOFOCUS_GAIN))
    }
}
