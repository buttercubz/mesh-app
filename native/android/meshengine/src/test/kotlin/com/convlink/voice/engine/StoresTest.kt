package com.convlink.voice.engine

import com.convlink.voice.engine.keys.AesGcm
import com.convlink.voice.engine.keys.KeyValueStore
import com.convlink.voice.engine.keys.SecureStore
import com.convlink.voice.engine.lab.LabFileStore
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.rules.TemporaryFolder
import javax.crypto.KeyGenerator

class MapStore : KeyValueStore {
    val map = HashMap<String, String>()
    override fun get(key: String) = map[key]
    override fun put(key: String, value: String?) {
        if (value == null) map.remove(key) else map[key] = value
    }

    override fun keys() = map.keys.toSet()
}

class SecureStoreTest {
    private val key = KeyGenerator.getInstance("AES").apply { init(256) }.generateKey()
    private val kv = MapStore()
    private val store = SecureStore(AesGcm({ key }), kv)

    @Test
    fun roundTripAndNothingInTheClear() {
        val secret = "epoch secret 0123456789".toByteArray()
        store.put("group/ab12", secret)
        assertArrayEquals(secret, store.get("group/ab12"))
        val stored = kv.map.values.single()
        assertFalse(stored.contains("epoch"))
        assertFalse(String(java.util.Base64.getDecoder().decode(stored)).contains("epoch"))
    }

    @Test
    fun keyNameIsBoundAsAad() {
        store.put("group/a", byteArrayOf(1, 2, 3))
        kv.map["v1:group/b"] = kv.map.getValue("v1:group/a")
        assertNull("a value moved to another key must not decrypt", store.get("group/b"))
    }

    @Test
    fun loadAllDropsUnreadableEntries() {
        store.put("group/a", byteArrayOf(1))
        store.put("lab/index", byteArrayOf(2))
        kv.map["v1:group/broken"] = "AAAA"
        val all = store.loadAll()
        assertEquals(listOf("group/a", "lab/index"), all.map { it.first })
        assertFalse(kv.map.containsKey("v1:group/broken"))
        store.put("group/a", null)
        assertNull(store.get("group/a"))
    }

    @Test
    fun noncesAreFresh() {
        store.put("k", byteArrayOf(9))
        val first = kv.map.getValue("v1:k")
        store.put("k", byteArrayOf(9))
        assertTrue(first != kv.map.getValue("v1:k"))
    }
}

class LabFileStoreTest {
    @get:Rule
    val tmp = TemporaryFolder()

    @Test
    fun appendsLinesSummaryAndDeletes() {
        val s = LabFileStore(tmp.newFolder("lab"))
        val run = "r-20261023-0912-ab12"
        s.append(run, """{"t":"labrun/1"}""")
        s.append(run, """{"t":"sample"}""")
        s.appendSummary(run, "metric,value")
        s.close(run)
        assertEquals(listOf("""{"t":"labrun/1"}""", """{"t":"sample"}"""), s.file(run).readLines())
        val listed = s.list().single()
        assertEquals(run, listed.runId)
        assertTrue(listed.summary!!.isFile)
        s.delete(run)
        assertTrue(s.list().isEmpty())
    }

    @Test
    fun runIdsCannotEscape() {
        assertFalse(LabFileStore.validate("../x"))
        assertFalse(LabFileStore.validate(".hidden"))
        assertFalse(LabFileStore.validate("a/b"))
        assertFalse(LabFileStore.validate(""))
        assertTrue(LabFileStore.validate("r-20261023-0912-ab12"))
    }

    @Test(expected = IllegalArgumentException::class)
    fun embeddedNewlineIsRefused() {
        LabFileStore(tmp.newFolder("lab2")).append("r-1", "a\nb")
    }
}
