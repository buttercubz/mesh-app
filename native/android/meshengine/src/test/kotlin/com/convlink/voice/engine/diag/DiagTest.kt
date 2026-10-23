package com.convlink.voice.engine.diag

import com.convlink.voice.core.FfiEngineConfig
import com.convlink.voice.core.FfiPlatform
import com.convlink.voice.core.FfiSessionStart
import com.convlink.voice.core.MeshEngine
import com.convlink.voice.engine.EngineHooks
import com.convlink.voice.engine.RecordingPorts
import org.json.JSONObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.rules.TemporaryFolder
import java.io.File
import java.util.concurrent.CopyOnWriteArrayList
import java.util.zip.ZipFile

class DiagFileStoreTest {
    @get:Rule
    val tmp = TemporaryFolder()

    private fun hdr(id: String, seg: Int) = """{"t":"hdr","v":1,"schema":"ridelog/1","seg":$seg,"sessionId":"$id"}"""

    private fun filler(n: Int) = List(n) { """{"t":"e","ts":$it,"src":"plat","ev":"pad","x":"${"a".repeat(40)}"}""" }

    @Test
    fun segmentsStartWithTheirHeaderAndClearDropsHeadlessLines() {
        val s = DiagFileStore(tmp.newFolder("diag"))
        val id = "d-20261022-2004-7k2q"
        s.append(id, 1, listOf(hdr(id, 1), """{"t":"e","ts":0.1,"src":"plat","ev":"bt"}"""))
        s.append(id, 1, listOf("""{"t":"mark","ts":2,"label":"lag"}"""))
        s.append(id, 2, listOf(hdr(id, 2)))
        val info = s.sessions().single()
        assertEquals(listOf("seg-0001.jsonl", "seg-0002.jsonl"), info.segments.map { it.name })
        val text = s.segmentFile(id, 1).readText()
        assertEquals(3, text.trim().lines().size)
        assertTrue(text.startsWith("{\"t\":\"hdr\""))
        s.clear()
        assertTrue(s.sessions().isEmpty())
        s.append(id, 2, listOf("""{"t":"s","ts":3}"""))
        assertTrue(s.sessions().isEmpty())
        assertEquals(1, s.droppedLines)
    }

    @Test
    fun pathsAreValidated() {
        val s = DiagFileStore(tmp.newFolder("diag"))
        for (bad in listOf("../x", "d-2026-1", "", "d-20261022-2004-7K2Q", "d-20261022-2004-7k2q/../x")) {
            assertTrue(runCatching { s.append(bad, 1, listOf("{}")) }.isFailure)
        }
        assertTrue(runCatching { s.append("d-20261022-2004-aaaa", 0, listOf("{}")) }.isFailure)
        assertTrue(runCatching { s.append("d-20261022-2004-aaaa", 1, listOf("a\nb")) }.isFailure)
    }

    @Test
    fun budgetEvictsOldestSessionsButNeverTheCurrentOne() {
        val dir = tmp.newFolder("diag")
        val s = DiagFileStore(dir, maxBytes = 20_000, currentMaxBytes = 12_000)
        val old = listOf("d-20261020-1000-aaaa", "d-20261021-1000-bbbb", "d-20261022-1000-cccc")
        old.forEachIndexed { i, id ->
            s.append(id, 1, listOf(hdr(id, 1)) + filler(80))
            s.segmentFile(id, 1).setLastModified(System.currentTimeMillis() - (10 - i) * 3_600_000L)
        }
        val cur = "d-20261022-2004-dddd"
        for (seg in 1..3) s.append(cur, seg, listOf(hdr(cur, seg)) + filler(60))
        s.enforceBudget()
        val ids = s.sessions().map { it.sessionId }
        assertTrue("current kept", cur in ids)
        assertFalse("oldest evicted", old[0] in ids)
        assertTrue(s.usage().bytes <= 20_000)
        val c = s.sessions().first { it.sessionId == cur }
        assertTrue(c.bytes <= 12_000)
        assertEquals("seg-0003.jsonl", c.segments.last().name)
    }

    @Test
    fun retentionDropsOldSessions() {
        val clock = 2_000_000_000_000L
        val s = DiagFileStore(tmp.newFolder("diag"), retentionMs = 86_400_000L, now = { clock })
        val id = "d-20261001-1000-aaaa"
        s.append(id, 1, listOf(hdr(id, 1)))
        s.segmentFile(id, 1).setLastModified(clock - 3 * 86_400_000L)
        val fresh = "d-20261022-1000-bbbb"
        s.append(fresh, 1, listOf(hdr(fresh, 1)))
        s.segmentFile(fresh, 1).setLastModified(clock)
        s.enforceBudget()
        assertEquals(listOf(fresh), s.sessions().map { it.sessionId })
    }

    @Test
    fun exportZipsTheLastSessionOrTheRing() {
        val s = DiagFileStore(tmp.newFolder("diag"))
        val out = tmp.newFolder("out")
        val (ringZip, n0) = s.exportZip(DiagFileStore.Scope.LAST, "9f3a71c2-n4711", "{}", listOf(hdr("d-20261022-2004-aaaa", 1)), out)
        assertEquals(0, n0)
        assertEquals(listOf("device.json", "diag/ring.jsonl"), ZipFile(ringZip).use { z -> z.entries().toList().map { it.name } })
        for ((i, id) in listOf("d-20261021-1000-aaaa", "d-20261022-1000-bbbb").withIndex()) {
            s.append(id, 1, listOf(hdr(id, 1)))
            s.segmentFile(id, 1).setLastModified(1_000_000_000_000L + i * 1000L)
        }
        val (last, n1) = s.exportZip(DiagFileStore.Scope.LAST, "x", """{"model":"SM-X520"}""", null, out)
        assertEquals(1, n1)
        assertEquals("convlink-diag-x.zip", last.name)
        ZipFile(last).use { z ->
            assertEquals(listOf("device.json", "diag/d-20261022-1000-bbbb/seg-0001.jsonl"), z.entries().toList().map { it.name })
            val seg = z.getInputStream(z.getEntry("diag/d-20261022-1000-bbbb/seg-0001.jsonl")).readBytes().toString(Charsets.UTF_8)
            assertEquals(hdr("d-20261022-1000-bbbb", 1) + "\n", seg)
        }
        val (all, n2) = s.exportZip(DiagFileStore.Scope.ALL, "../all", "{}", null, out)
        assertEquals(2, n2)
        assertEquals("convlink-diag-all.zip", all.name)
    }

    @Test
    fun notesAreCompactJsonWithoutNames() {
        val n = DiagNote.perm("microphone", "granted", "check", "ContextCompat.checkSelfPermission(RECORD_AUDIO)")
        assertEquals(
            """{"name":"microphone","state":"granted","via":"check","api":"ContextCompat.checkSelfPermission(RECORD_AUDIO)"}""",
            n.fieldsJson,
        )
        val q = DiagNote("x", listOf("s" to "a\"b\n" + "z".repeat(100), "i" to listOf(1, 2), "d" to 0.5, "n" to null))
        assertTrue(q.fieldsJson.startsWith("""{"s":"a\"b\nzz"""))
        assertTrue(q.fieldsJson.endsWith(""""i":[1,2],"d":0.500,"n":null}"""))
        assertEquals("connectedDevice,microphone", DiagNote.fgsTypeNames(0x10 or 0x80))
        assertEquals(DiagNote.os("coldStart"), DiagNote.os("coldStart"))
    }

    @Test
    fun defaultPreferenceFollowsTheFieldTestPeriod() {
        assertTrue(DiagPreference.defaultEnabled("0.1.0 (12)", false))
        assertFalse(DiagPreference.defaultEnabled("1.0.0 (40)", false))
        assertTrue(DiagPreference.defaultEnabled("1.2.0 (40)", true))
    }
}

/** The real Rust engine (JNA, host dylib) writing ridelog/1 through the hooks into a store. */
class DiagEngineTest {
    @get:Rule
    val tmp = TemporaryFolder()

    private class DiagPorts(val writer: DiagWriter) : EngineHooks.Ports by RecordingPorts() {
        val batches = CopyOnWriteArrayList<Pair<String, Int>>()

        override fun diagWrite(sessionId: String, seg: Int, lines: List<String>) {
            batches.add(sessionId to seg)
            writer.write(sessionId, seg, lines)
        }
    }

    @Test
    fun notesAndALabSessionReachTheStore() {
        val store = DiagFileStore(tmp.newFolder("diag"))
        val writer = DiagWriter(store) {}
        val ports = DiagPorts(writer)
        val engine = MeshEngine(
            FfiEngineConfig(
                platform = FfiPlatform.ANDROID,
                appVersion = 1u,
                rngSeed = ByteArray(32) { (it + 7).toByte() },
                deviceModel = "samsung SM-X520",
                deviceOs = "Android 16",
                appLabel = "0.1.0 (1)",
                persisted = emptyList(),
            ),
            EngineHooks(ports),
        )
        try {
            engine.diagConfigure(true, true)
            val perm = DiagNote.perm("microphone", "granted", "check", "ContextCompat.checkSelfPermission(RECORD_AUDIO)")
            assertTrue(engine.diagNote("plat", perm.ev, perm.fieldsJson))
            assertFalse(engine.diagNote("core", "x", "{}"))
            engine.start(FfiSessionStart.Lab(1357.toUShort()))
            assertTrue(engine.diagMark("crackle"))
            Thread.sleep(1_300)
            engine.stop()
            engine.diagFlush()
            writer.drain()
            val lines = store.sessions().flatMap { s -> s.segments.flatMap { it.readLines() } }.map { JSONObject(it) }
            assertTrue(lines.any { it.optString("ev") == "perm" && it.optString("api").startsWith("ContextCompat") })
            val lab = lines.firstOrNull { it.optString("t") == "hdr" && it.optString("rideKey").startsWith("lab-") }
            assertTrue("Lab header with ride key", lab != null)
            assertEquals("android", lab!!.getJSONObject("device").getString("platform"))
            assertTrue(lines.any { it.optString("t") == "mark" && it.optString("label") == "crackle" })
            assertTrue(lines.any { it.optString("t") == "s" })
            assertTrue(lines.any { it.optString("ev") == "engineStop" })
            assertTrue(ports.batches.all { (id, seg) -> DiagFileStore.validate(id) && seg >= 1 })
            assertTrue(engine.diagStatus().enabled)
            val ring = engine.diagSnapshotRing()
            assertTrue(ring.isNotEmpty() && ring[0].startsWith("{\"t\":\"hdr\""))
            assertTrue(File(store.directory, store.sessions().first().sessionId).isDirectory)
        } finally {
            engine.destroy()
        }
    }
}
