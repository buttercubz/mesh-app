package com.convlink.voice.engine.ble

import com.convlink.voice.core.FfiAdvData
import com.convlink.voice.core.FfiDiscovery
import com.convlink.voice.core.FfiLinkDownReason
import com.convlink.voice.core.FfiLinkInfo
import com.convlink.voice.core.FfiLinkKind
import com.convlink.voice.core.FfiLinkRole
import com.convlink.voice.core.FfiSendOutcome
import com.convlink.voice.core.FfiTransport
import com.convlink.voice.engine.LinkRegistry
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import java.io.ByteArrayOutputStream
import java.io.Closeable
import java.io.OutputStream
import java.io.PipedInputStream
import java.io.PipedOutputStream
import java.util.concurrent.CopyOnWriteArrayList
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit

class RecordingSink : LinkSink {
    val ups = CopyOnWriteArrayList<Int>()
    val downs = CopyOnWriteArrayList<Pair<Int, FfiLinkDownReason>>()
    val data = CopyOnWriteArrayList<Pair<Int, ByteArray>>()
    val writable = CopyOnWriteArrayList<Int>()
    val failed = CopyOnWriteArrayList<Pair<Int, FfiLinkDownReason>>()

    override fun linkUp(link: MeshLink, info: FfiLinkInfo) {
        ups.add(link.id)
    }

    override fun linkDown(id: Int, reason: FfiLinkDownReason) {
        downs.add(id to reason)
    }

    override fun linkData(id: Int, bytes: ByteArray) {
        data.add(id to bytes)
    }

    override fun linkWritable(id: Int) {
        writable.add(id)
    }

    override fun linkMetrics(id: Int, rssi: Short?, attMtu: UShort?) {
        metrics.add(Triple(id, rssi, attMtu))
    }
    override fun discovered(d: FfiDiscovery) = Unit
    val hellos = CopyOnWriteArrayList<Pair<Int, ByteArray>>()
    val metrics = CopyOnWriteArrayList<Triple<Int, Short?, UShort?>>()

    override fun helloRead(discId: Int, bytes: ByteArray) {
        hellos.add(discId to bytes)
    }

    override fun connectFailed(discId: Int, reason: FfiLinkDownReason) {
        failed.add(discId to reason)
    }
}

private class FakeLink(override val id: Int) : MeshLink {
    override val kind = FfiLinkKind.BLE_GATT
    val sent = CopyOnWriteArrayList<ByteArray>()
    override fun send(bytes: ByteArray): FfiSendOutcome {
        sent.add(bytes)
        return FfiSendOutcome.ACCEPTED
    }

    override fun close(reason: Int) = Unit
}

class LinkRegistryTest {
    @Test
    fun idsAreUniqueAndDownIsReportedOnce() {
        val sink = RecordingSink()
        val r = LinkRegistry(sink)
        val a = r.allocateId()
        val l = FakeLink(a)
        r.up(l, gattInfo(FfiLinkRole.RESPONDER, 185, BleConfig.DESIGN, null))
        assertTrue(r.allocateId() != a)
        assertEquals(FfiSendOutcome.ACCEPTED, r.send(a, byteArrayOf(1)))
        r.data(a, byteArrayOf(9))
        assertTrue(r.down(a, FfiLinkDownReason.PeerClosed))
        assertFalse(r.down(a, FfiLinkDownReason.PeerClosed))
        r.data(a, byteArrayOf(9)) // dropped
        assertEquals(listOf(a), sink.ups)
        assertEquals(1, sink.downs.size)
        assertEquals(1, sink.data.size)
        assertEquals(FfiSendOutcome.NO_LINK, r.send(a, byteArrayOf(1)))
    }

    @Test
    fun radioOffDropsBleLinks() {
        val sink = RecordingSink()
        val r = LinkRegistry(sink)
        val ids = (1..3).map { r.allocateId().also { id -> r.up(FakeLink(id), gattInfo(FfiLinkRole.INITIATOR, 247, BleConfig.DESIGN, id)) } }
        val dropped = r.dropAll(setOf(FfiLinkKind.BLE_GATT, FfiLinkKind.BLE_L2CAP), FfiLinkDownReason.RadioOff)
        assertEquals(ids, dropped)
        assertTrue(sink.downs.all { it.second == FfiLinkDownReason.RadioOff })
        assertEquals(0, r.count)
    }

    @Test
    fun gattInfoClampsMaxFrame() {
        val i = gattInfo(FfiLinkRole.INITIATOR, 517, BleConfig.DESIGN, 4)
        assertEquals(514.toUShort(), i.maxFrame)
        assertEquals(517.toUShort(), i.attMtu)
        assertEquals(20.toUShort(), gattInfo(FfiLinkRole.INITIATOR, 23, BleConfig.DESIGN, null).maxFrame)
        assertEquals(FfiLinkKind.BLE_L2CAP, l2capInfo(FfiLinkRole.RESPONDER, 1024, null).kind)
    }
}

class L2capLinkTest {
    private fun closer(latch: CountDownLatch) = Closeable { latch.countDown() }

    @Test
    fun readerDeliversChunksAndEofIsPeerClosed() {
        val sink = RecordingSink()
        val r = LinkRegistry(sink)
        val peer = PipedOutputStream()
        val input = PipedInputStream(peer, 4096)
        val out = ByteArrayOutputStream()
        val closed = CountDownLatch(1)
        val id = r.allocateId()
        val link = L2capLink(id, input, out, closer(closed), r)
        r.up(link, l2capInfo(FfiLinkRole.RESPONDER, 1024, null))
        link.start()
        peer.write(byteArrayOf(1, 2, 3))
        peer.flush()
        assertEquals(FfiSendOutcome.ACCEPTED, link.send(byteArrayOf(7, 8)))
        waitUntil { sink.data.isNotEmpty() && out.size() == 2 }
        assertArrayEquals(byteArrayOf(1, 2, 3), sink.data.first().second)
        assertArrayEquals(byteArrayOf(7, 8), out.toByteArray())
        peer.close()
        assertTrue(closed.await(2, TimeUnit.SECONDS))
        waitUntil { sink.downs.isNotEmpty() }
        assertEquals(FfiLinkDownReason.PeerClosed, sink.downs.single().second)
        assertEquals(FfiSendOutcome.NO_LINK, link.send(byteArrayOf(1)))
    }

    @Test
    fun fullQueueAnswersWouldBlockThenWritable() {
        val sink = RecordingSink()
        val r = LinkRegistry(sink)
        val gate = CountDownLatch(1)
        val written = ByteArrayOutputStream()
        val slow = object : OutputStream() {
            override fun write(b: Int) {
                gate.await()
                written.write(b)
            }

            override fun write(b: ByteArray, off: Int, len: Int) {
                gate.await()
                written.write(b, off, len)
            }
        }
        val id = r.allocateId()
        val link = L2capLink(id, PipedInputStream(PipedOutputStream()), slow, Closeable { }, r, maxQueuedBytes = 100)
        r.up(link, l2capInfo(FfiLinkRole.INITIATOR, 1024, 1))
        link.start()
        assertEquals(FfiSendOutcome.ACCEPTED, link.send(ByteArray(60)))
        assertEquals(FfiSendOutcome.ACCEPTED, link.send(ByteArray(40)))
        assertEquals(FfiSendOutcome.WOULD_BLOCK, link.send(ByteArray(10)))
        gate.countDown()
        waitUntil { sink.writable.isNotEmpty() }
        assertEquals(listOf(id), sink.writable.distinct())
        waitUntil { link.pendingBytes == 0 }
        assertEquals(100, written.size())
        link.close(0)
        waitUntil { sink.downs.isNotEmpty() }
        assertEquals(FfiLinkDownReason.LocalClose, sink.downs.first().second)
    }

    private fun waitUntil(cond: () -> Boolean) {
        val end = System.currentTimeMillis() + 3_000
        while (!cond()) {
            check(System.currentTimeMillis() < end) { "timed out" }
            Thread.sleep(5)
        }
    }
}

/** Discovery of a FfiDiscovery is structural (used by the engine sink). */
class DiscoveryShapeTest {
    @Test
    fun transportIsBle() {
        val d = DiscoveryParser.discovery(1, ByteArray(9) { 1 }, -40)
        assertEquals(FfiTransport.BLE, d.transport)
        assertEquals(9, (d.adv as FfiAdvData.ServiceData).bytes.size)
    }
}
