package com.convlink.voice.engine

import com.convlink.voice.core.FfiAudioCommand
import com.convlink.voice.core.FfiDiscoveryMode
import com.convlink.voice.core.FfiEngineConfig
import com.convlink.voice.core.FfiException
import com.convlink.voice.core.FfiLogLevel
import com.convlink.voice.core.FfiNotification
import com.convlink.voice.core.FfiPlatform
import com.convlink.voice.core.FfiPromptRequest
import com.convlink.voice.core.FfiRadioCommand
import com.convlink.voice.core.FfiRuntimeStatus
import com.convlink.voice.core.FfiSendOutcome
import com.convlink.voice.core.FfiSessionStart
import com.convlink.voice.core.MeshEngine
import com.convlink.voice.core.apiVersion
import com.convlink.voice.core.bleConstants
import com.convlink.voice.core.protocolVersion
import com.convlink.voice.core.rtAbiVersion
import com.convlink.voice.engine.ble.BleConfig
import org.json.JSONObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import java.util.concurrent.CopyOnWriteArrayList
import java.util.concurrent.TimeUnit

/** Fake ports recording what the real engine asks of the platform. */
class RecordingPorts : EngineHooks.Ports {
    val radio = CopyOnWriteArrayList<FfiRadioCommand>()
    val audio = CopyOnWriteArrayList<FfiAudioCommand>()
    val events = CopyOnWriteArrayList<String>()
    val persisted = java.util.concurrent.ConcurrentHashMap<String, ByteArray>()
    val logs = CopyOnWriteArrayList<String>()

    override fun radio(cmd: FfiRadioCommand) {
        radio.add(cmd)
    }

    override fun linkSend(link: Int, bytes: ByteArray) = FfiSendOutcome.NO_LINK

    override fun audio(cmd: FfiAudioCommand) {
        audio.add(cmd)
    }

    override fun uiEvent(json: String) {
        events.add(json)
    }

    override fun persist(key: String, value: ByteArray?) {
        if (value == null) persisted.remove(key) else persisted[key] = value
    }

    override fun notify(n: FfiNotification) = Unit

    override fun labWrite(runId: String, line: String, summary: Boolean) = Unit

    override fun labDelete(runId: String) = Unit

    override fun renderPrompts(reqs: List<FfiPromptRequest>) = Unit

    override fun log(level: FfiLogLevel, msg: String) {
        logs.add(msg)
    }
}

/**
 * Generated UniFFI bindings run against the host libmesh_ffi.dylib through JNA
 * (`jna.library.path` = target/release; DESIGN §13 Kotlin row).
 */
class BindingsSmokeTest {
    private fun engine(ports: RecordingPorts) = MeshEngine(
        FfiEngineConfig(
            platform = FfiPlatform.ANDROID,
            appVersion = 1u,
            rngSeed = ByteArray(32) { it.toByte() },
            deviceModel = "JVM test",
            deviceOs = "Android 16",
            appLabel = "0.1.0 (1)",
            persisted = emptyList(),
        ),
        EngineHooks(ports),
    )

    private fun waitFor(timeoutMs: Long = 5_000, cond: () -> Boolean): Boolean {
        val end = System.nanoTime() + TimeUnit.MILLISECONDS.toNanos(timeoutMs)
        while (System.nanoTime() < end) {
            if (cond()) return true
            Thread.sleep(10)
        }
        return cond()
    }

    @Test
    fun versionsMatchTheContracts() {
        assertEquals(EngineHost.API_MAJOR, apiVersion().toInt() shr 8)
        assertEquals(1u, rtAbiVersion())
        assertTrue(protocolVersion().toInt() >= 1)
    }

    @Test
    fun bleConstantsMatchDesign() {
        val c = BleConfig.from(bleConstants())
        val d = BleConfig.DESIGN
        assertEquals(d.service, c.service)
        assertEquals(d.hello, c.hello)
        assertEquals(d.rx, c.rx)
        assertEquals(d.tx, c.tx)
        assertEquals(16, c.helloLen)
        assertEquals(517, c.preferredAttMtu)
        assertEquals(250L, c.advIntervalFormingMs)
        assertEquals(1_000L, c.advIntervalStableMs)
        assertEquals(9 * 60_000L, c.scanRestartMs)
        assertEquals(5, c.scanQuotaStarts)
        assertEquals(30_000L, c.scanQuotaWindowMs)
    }

    @Test
    fun idleEngineAnswersCommands() {
        val ports = RecordingPorts()
        engine(ports).use { e ->
            assertEquals(FfiRuntimeStatus.IDLE, e.status())
            val r = JSONObject(e.uiCommand("""{"type":"listGroups"}"""))
            assertTrue(r.toString(), r.getBoolean("ok"))
            val bad = JSONObject(e.uiCommand("not json"))
            assertEquals(false, bad.getBoolean("ok"))
        }
    }

    @Test
    fun labSessionDrivesTheRadioAndStops() {
        val ports = RecordingPorts()
        engine(ports).use { e ->
            e.start(FfiSessionStart.Lab(1234u))
            assertEquals(FfiRuntimeStatus.RUNNING, e.status())
            assertTrue("no discovery", waitFor {
                ports.radio.any { it is FfiRadioCommand.SetDiscovery && it.mode != FfiDiscoveryMode.OFF }
            })
            val disc = ports.radio.filterIsInstance<FfiRadioCommand.SetDiscovery>().first { it.mode != FfiDiscoveryMode.OFF }
            // Android advertises the 9-byte scan-response service data (§9.4).
            assertEquals(9, disc.adv?.serviceData?.size)
            assertTrue("no HELLO", waitFor { ports.radio.any { it is FfiRadioCommand.SetHello } })
            val hello = ports.radio.filterIsInstance<FfiRadioCommand.SetHello>().last()
            assertEquals(16, hello.bytes.size)
            assertEquals(0xC1, hello.bytes[0].toInt() and 0xFF)
            e.stop()
            assertTrue(waitFor { e.status() != FfiRuntimeStatus.RUNNING })
        }
    }

    @Test
    fun demoStartIsReportedAsAnFfiError() {
        val ports = RecordingPorts()
        engine(ports).use { e ->
            try {
                e.start(FfiSessionStart.Demo)
                e.stop()
            } catch (ex: FfiException) {
                // Builds without the sim world answer DemoUnavailable (WP-12 deviation).
                assertTrue(ex is FfiException.DemoUnavailable)
            }
        }
    }
}
