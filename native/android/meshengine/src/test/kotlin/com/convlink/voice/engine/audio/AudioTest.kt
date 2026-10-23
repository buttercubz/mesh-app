package com.convlink.voice.engine.audio

import android.media.AudioDeviceInfo
import com.convlink.voice.core.FfiAudioCommand
import com.convlink.voice.core.FfiAudioIoState
import com.convlink.voice.core.FfiAudioRoute
import com.convlink.voice.core.FfiAudioRouteKind
import com.convlink.voice.core.FfiAudioSessionKind
import com.convlink.voice.core.FfiAudioSessionProfile
import com.convlink.voice.core.FfiRoutePreference
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import java.util.concurrent.CopyOnWriteArrayList

class FakeIo : AudioIo {
    var opens = CopyOnWriteArrayList<List<Any>>()
    var closes = 0
    var capturing = false
    var failNext = 0
    var captureCallbacks = 0L
    var lastFlags = 0
    override var onStreamError: ((Boolean, Int) -> Unit)? = null

    override fun open(capture: Boolean, render: Boolean, inputDeviceId: Int, outputDeviceId: Int, bypassVp: Boolean): Int {
        if (failNext > 0) {
            failNext--
            return -889
        }
        opens.add(listOf(capture, render, bypassVp))
        return 0
    }

    override fun close() {
        closes++
    }

    override fun setCaptureEnabled(enabled: Boolean) {
        capturing = enabled
    }

    override fun stats() = RtStats(0, 0, 0, 0, captureCallbacks, 0, -90f, -90f, 1)

    override fun setFlags(flags: Int) {
        lastFlags = flags
    }
}

private class FakeStrategy : AudioSessionStrategy {
    override val name = "fake"
    override var listener: AudioRouteController.Listener? = null
    var active = false
    override fun activate(preference: FfiRoutePreference, capture: Boolean): Boolean {
        active = true
        return true
    }

    override fun setPreference(preference: FfiRoutePreference) = Unit
    override fun deactivate() {
        active = false
    }
}

class PlatformAudioTest {
    private val states = CopyOnWriteArrayList<FfiAudioIoState>()
    private val io = FakeIo()
    private val strategy = FakeStrategy()
    private var now = 0L
    private val audio = PlatformAudio(io, strategy, object : PlatformAudio.Callbacks {
        override fun audioIo(state: FfiAudioIoState) {
            states.add(state)
        }

        override fun audioRoute(route: FfiAudioRoute) = Unit
        override fun onCall(active: Boolean) = Unit
        override fun log(msg: String) = Unit
    }, clock = { now })

    private val goLive = GesturePlan.make(GestureCommand.GoLive, shareDistance = false)

    @Test
    fun gestureOpensStreamsMutedAndStartCaptureOnlyUnmutes() {
        assertTrue(audio.prepareGesture(goLive))
        assertTrue(strategy.active)
        assertEquals(listOf<Any>(true, true, false), io.opens.single())
        assertFalse("capture starts muted", io.capturing)
        audio.handle(FfiAudioCommand.StartCapture)
        assertTrue(io.capturing)
        // A late or duplicated StartCapture is harmless: no reopen.
        audio.handle(FfiAudioCommand.StartCapture)
        assertEquals(1, io.opens.size)
        // The same gesture again opens nothing new.
        assertFalse(audio.prepareGesture(goLive))
    }

    @Test
    fun rollbackClosesTheSession() {
        audio.prepareGesture(goLive)
        audio.rollbackGesture()
        assertFalse(strategy.active)
        assertFalse(audio.isLive)
    }

    @Test
    fun failedOpenThrows() {
        io.failNext = 1
        try {
            audio.prepareGesture(goLive)
            error("expected failure")
        } catch (_: IllegalStateException) {
        }
        assertFalse(audio.isLive)
    }

    @Test
    fun watchdogStallRestartsThenReportsCaptureLost() {
        audio.prepareGesture(goLive)
        audio.handle(FfiAudioCommand.StartCapture)
        io.captureCallbacks = 10
        now = 100
        audio.tick() // counter moved: re-armed
        now = 1_700
        audio.tick() // stalled > 1.5 s → restart OK
        // Running (StartCapture after the gesture), then the restart.
        assertEquals(listOf(FfiAudioIoState.Running, FfiAudioIoState.Restarting, FfiAudioIoState.Running), states.toList())
        assertEquals(2, io.opens.size)
        assertTrue("capture stays connected after a restart", io.capturing)
        io.failNext = 1
        now = 3_400
        audio.tick() // stalled again → restart fails → CaptureLost
        assertEquals(FfiAudioIoState.CaptureLost, states.last())
        assertFalse(audio.isLive)
    }

    @Test
    fun outputOnlyFallbackReopensRenderOnly() {
        audio.prepareGesture(goLive)
        audio.handle(FfiAudioCommand.OutputOnlyFallback(true))
        assertEquals(listOf<Any>(false, true, false), io.opens.last())
        audio.handle(FfiAudioCommand.OutputOnlyFallback(false))
        assertEquals(listOf<Any>(true, true, false), io.opens.last())
    }

    @Test
    fun openSessionWithoutGestureAndClose() {
        val profile = FfiAudioSessionProfile(FfiAudioSessionKind.LAB_RENDER, capture = false, mixable = false,
            locationAnchor = false, bypassVoiceProcessing = false)
        audio.handle(FfiAudioCommand.OpenSession(profile))
        assertEquals(listOf<Any>(false, true, false), io.opens.single())
        assertEquals(FfiAudioIoState.Running, states.last())
        audio.handle(FfiAudioCommand.CloseSession)
        assertEquals(FfiAudioIoState.Stopped, states.last())
        assertFalse(strategy.active)
    }

    private val rideProfile = FfiAudioSessionProfile(FfiAudioSessionKind.RIDE, capture = true, mixable = false,
        locationAnchor = false, bypassVoiceProcessing = false)

    @Test
    fun goLiveGestureReportsRunningOnceTheCoreOpensTheSession() {
        // Field bug (2026-10-22, Samsung): the gesture opened the streams, OpenSession saw
        // them open and reported nothing, so the core never set captureOk ("mic paused").
        audio.prepareGesture(goLive)
        assertTrue("nothing is reported before the command reaches the core", states.isEmpty())
        audio.handle(FfiAudioCommand.OpenSession(rideProfile))
        audio.handle(FfiAudioCommand.StartCapture)
        assertEquals(listOf<FfiAudioIoState>(FfiAudioIoState.Running), states.toList())
        assertEquals("no reopen", 1, io.opens.size)
    }

    @Test
    fun audioCheckGestureReportsRunning() {
        audio.prepareGesture(GesturePlan.make(GestureCommand.AudioCheckStart("tone"), shareDistance = false))
        val check = FfiAudioSessionProfile(FfiAudioSessionKind.CHECK, capture = true, mixable = false,
            locationAnchor = false, bypassVoiceProcessing = false)
        audio.handle(FfiAudioCommand.OpenSession(check))
        assertEquals(listOf<FfiAudioIoState>(FfiAudioIoState.Running), states.toList())
    }

    @Test
    fun resumeMicAfterCaptureLossReportsRestored() {
        audio.prepareGesture(goLive)
        audio.handle(FfiAudioCommand.OpenSession(rideProfile))
        audio.handle(FfiAudioCommand.StartCapture)
        // Stream error whose reopen fails → CaptureLost; the core asks for output only.
        io.failNext = 1
        io.onStreamError!!.invoke(true, -899)
        assertEquals(FfiAudioIoState.CaptureLost, states.last())
        audio.handle(FfiAudioCommand.OutputOnlyFallback(true))
        // Resume tap: the gesture reopens capture; the core answers with StartCapture only.
        val resume = GesturePlan.make(GestureCommand.ResumeMic, shareDistance = false)
        assertTrue(audio.prepareGesture(resume))
        audio.handle(FfiAudioCommand.OutputOnlyFallback(false))
        audio.handle(FfiAudioCommand.StartCapture)
        assertEquals(FfiAudioIoState.Restored, states.last())
        assertEquals(listOf<Any>(true, true, false), io.opens.last())
    }

    @Test
    fun healthyStateIsNotReportedTwiceAndIsReportedAgainAfterAReopen() {
        audio.prepareGesture(goLive)
        audio.handle(FfiAudioCommand.OpenSession(rideProfile))
        audio.handle(FfiAudioCommand.StartCapture)
        audio.handle(FfiAudioCommand.StartCapture)
        assertEquals(1, states.size)
        audio.handle(FfiAudioCommand.CloseSession)
        assertEquals(FfiAudioIoState.Stopped, states.last())
        audio.prepareGesture(goLive)
        audio.handle(FfiAudioCommand.OpenSession(rideProfile))
        assertEquals(FfiAudioIoState.Running, states.last())
    }

    @Test
    fun startCaptureWhileOutputOnlyReportsNothing() {
        audio.prepareGesture(goLive)
        audio.handle(FfiAudioCommand.OpenSession(rideProfile))
        io.failNext = 1
        io.onStreamError!!.invoke(true, -899)
        audio.handle(FfiAudioCommand.OutputOnlyFallback(true))
        val before = states.size
        audio.handle(FfiAudioCommand.StartCapture)
        assertEquals(before, states.size)
    }

    @Test
    fun streamErrorReopens() {
        audio.prepareGesture(goLive)
        io.onStreamError!!.invoke(true, -899)
        assertEquals(2, io.opens.size)
        assertEquals(FfiAudioIoState.Running, states.last())
    }
}

class GesturePlanTest {
    @Test
    fun labPaths() {
        val render = GesturePlan.make(GestureCommand.LabStart("T1", "renderOnly", "audioLocation"), false)
        assertEquals(AudioPath.RENDER_ONLY, render.path)
        assertFalse(render.microphone)
        assertTrue(render.wantsLocation)
        assertEquals(AudioPath.BYPASS_VP, GesturePlan.make(GestureCommand.LabStart("T5a", "full", "audio"), false).path)
        assertTrue(GesturePlan.make(GestureCommand.GoLive, true).wantsLocation)
        assertTrue(GesturePlan.make(GestureCommand.ResumeMic, false).resume)
        assertFalse(GesturePlan.make(GestureCommand.AudioCheckStart("tone"), true).wantsLocation)
    }
}

class MuteWatchdogTest {
    @Test
    fun firesOncePerStall() {
        val w = MuteWatchdog()
        assertFalse(w.check(1, 0))
        w.arm(5, 0)
        assertFalse(w.check(5, 1_500))
        assertTrue(w.check(5, 1_501))
        assertFalse(w.check(5, 3_000))
        assertFalse(w.check(6, 3_100))
        assertTrue(w.check(6, 4_700))
        w.disarm()
        assertFalse(w.isArmed)
    }
}

class RtFlagsTest {
    @Test
    fun bitsAndLatchEdge() {
        val writes = mutableListOf<Int>()
        val f = RtFlags { writes.add(it) }
        f.setPttDown(true)
        assertTrue(f.toggleHardMute())
        f.latchToggle()
        f.setPttDown(false)
        assertEquals(listOf(0b0001, 0b0011, 0b1011, 0b0010), writes)
        assertTrue(f.hardMuted)
        f.setBuiltinMicMute(true)
        assertEquals(0b0110, f.value)
    }
}

class RouteSelectorTest {
    private val speaker = RouteDevice(1, AudioDeviceInfo.TYPE_BUILTIN_SPEAKER)
    private val sco = RouteDevice(2, AudioDeviceInfo.TYPE_BLUETOOTH_SCO, intArrayOf(8_000))
    private val ble = RouteDevice(3, AudioDeviceInfo.TYPE_BLE_HEADSET)
    private val wired = RouteDevice(4, AudioDeviceInfo.TYPE_WIRED_HEADSET)

    @Test
    fun priorityOrder() {
        assertEquals(ble, RouteSelector.choose(listOf(speaker, sco, wired, ble), FfiRoutePreference.AUTO))
        assertEquals(sco, RouteSelector.choose(listOf(speaker, wired, sco), FfiRoutePreference.AUTO))
        assertEquals(wired, RouteSelector.choose(listOf(speaker, wired), FfiRoutePreference.HEADSET))
        assertEquals(speaker, RouteSelector.choose(listOf(speaker), FfiRoutePreference.AUTO))
        assertEquals(speaker, RouteSelector.choose(listOf(sco, speaker), FfiRoutePreference.SPEAKER))
        assertNull(RouteSelector.choose(emptyList(), FfiRoutePreference.AUTO))
    }

    @Test
    fun routeKinds() {
        assertEquals(FfiAudioRouteKind.HFP_NARROWBAND, RouteSelector.kind(sco))
        assertEquals(FfiAudioRouteKind.HFP_WIDEBAND, RouteSelector.kind(sco.copy(sampleRates = intArrayOf(8_000, 16_000))))
        assertEquals(FfiAudioRouteKind.LE_AUDIO, RouteSelector.kind(ble))
        val r = RouteSelector.route(sco, listOf(sco, speaker), capture = true)
        assertEquals(8_000u, r.inputRate)
        assertTrue(r.headsetPresent)
        assertEquals(0u, RouteSelector.route(speaker, listOf(speaker), capture = false).inputRate)
    }
}

class WavTest {
    private fun wav(rate: Int, channels: Int, samples: ShortArray): ByteArray {
        val data = java.nio.ByteBuffer.allocate(44 + samples.size * 2).order(java.nio.ByteOrder.LITTLE_ENDIAN)
        data.put("RIFF".toByteArray()).putInt(36 + samples.size * 2).put("WAVE".toByteArray())
        data.put("fmt ".toByteArray()).putInt(16).putShort(1).putShort(channels.toShort()).putInt(rate)
            .putInt(rate * channels * 2).putShort((channels * 2).toShort()).putShort(16)
        data.put("data".toByteArray()).putInt(samples.size * 2)
        samples.forEach { data.putShort(it) }
        return data.array()
    }

    @Test
    fun parsesAndResamples() {
        val pcm = Wav.toMono16k(wav(32_000, 1, ShortArray(320) { 1000 }))!!
        assertEquals(160, pcm.size)
        assertTrue(pcm.all { it.toInt() == 1000 })
        val stereo = Wav.toMono16k(wav(16_000, 2, shortArrayOf(100, 300, 100, 300)))!!
        assertEquals(listOf<Short>(200, 200), stereo.toList())
        assertNull(Wav.parse(ByteArray(10)))
    }
}
