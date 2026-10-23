package com.convlink.voice.engine

import com.convlink.voice.engine.audio.CommandJson
import com.convlink.voice.engine.audio.GestureCommand
import com.convlink.voice.engine.notify.EventDigest
import com.convlink.voice.engine.notify.IntercomState
import org.json.JSONArray
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import java.io.File

/** Repository paths for tests (set by build.gradle.kts). */
object Repo {
    val root = File(System.getProperty("meshengine.repoRoot") ?: "../../..")
    fun file(path: String) = File(root, path)
}

/** Replays the shared mesh-api fixtures (crates/mesh-api/fixtures) against the Kotlin side. */
class ContractFixturesTest {
    @Test
    fun gestureClassificationMatchesTheSharedFixture() {
        val cases = JSONArray(Repo.file("crates/mesh-api/fixtures/commands.json").readText())
        assertTrue(cases.length() > 10)
        for (i in 0 until cases.length()) {
            val c = cases.getJSONObject(i)
            val json = c.getJSONObject("command").toString()
            assertEquals(c.getString("name"), c.getBoolean("requiresGesture"), GestureCommand.requiresGesture(json))
        }
    }

    @Test
    fun invalidJsonIsNotGestureGated() {
        assertNull(GestureCommand.classify("{"))
        assertNull(GestureCommand.classify("""{"type":"labStart","bench":"T0","params":{}}"""))
        assertFalse(CommandJson.isOk("nope"))
        assertTrue(CommandJson.isOk(CommandJson.error("x").replace("false", "true")))
        assertEquals("useGestureCommand", org.json.JSONObject(CommandJson.error("useGestureCommand")).getString("code"))
    }

    @Test
    fun eventDigestFollowsSnapshotsAndLabEvents() {
        val events = JSONArray(Repo.file("crates/mesh-api/fixtures/events.json").readText())
        val byName = (0 until events.length()).associate {
            val e = events.getJSONObject(it)
            e.getString("name") to e.getJSONObject("event").toString()
        }
        val d = EventDigest()
        assertTrue(d.apply(byName.getValue("snapshot ride")))
        assertTrue(d.live)
        assertTrue(d.shareDistance)
        assertEquals(IntercomState.Stage.LIVE, d.state(muted = false, captureLost = false).stage)
        d.apply(byName.getValue("snapshot lobby degraded"))
        assertEquals(IntercomState.Stage.LOBBY, d.state(false, false).stage)
        d.apply(byName.getValue("labProgress"))
        assertEquals(IntercomState.Stage.LAB, d.state(false, false).stage)
        d.apply(byName.getValue("labFinished"))
        assertNull(d.labRunId)
        assertFalse(d.apply("not json"))
        d.commandAccepted(org.json.JSONObject("""{"type":"setShareDistance","enabled":false}"""))
        assertFalse(d.shareDistance)
    }
}
