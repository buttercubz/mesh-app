package com.convlink.voice.engine.ble

import com.convlink.voice.core.FfiLinkDownReason
import com.convlink.voice.core.FfiLinkPriority
import com.convlink.voice.core.FfiSendOutcome
import com.convlink.voice.engine.LinkRegistry
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/** Runs posts at once; delayed runnables wait for [runDelayed]. */
class ManualScheduler : Scheduler {
    val delayed = mutableListOf<Pair<Runnable, Long>>()
    override fun post(r: Runnable) = r.run()
    override fun postDelayed(r: Runnable, ms: Long) {
        delayed.add(r to ms)
    }

    override fun cancel(r: Runnable) {
        delayed.removeAll { it.first === r }
    }

    fun runDelayed() {
        val due = delayed.toList()
        delayed.clear()
        due.forEach { it.first.run() }
    }
}

class FakeGatt : GattOps {
    val calls = mutableListOf<String>()
    val writes = mutableListOf<ByteArray>()
    var dataPath = true
    var acceptWrites = true
    var mtuRequested = 0
    override val hasDataPath: Boolean get() = dataPath
    override fun setPreferredPhy2M() {
        calls.add("phy2m")
    }

    override fun requestMtu(mtu: Int): Boolean {
        mtuRequested = mtu
        calls.add("mtu")
        return true
    }

    override fun discoverServices(): Boolean = calls.add("discover")
    override fun resolveService(): Boolean = calls.add("resolve")
    override fun readHello(): Boolean = calls.add("readHello")
    override fun enableTxNotifications(): Boolean = calls.add("subscribeTx")
    override fun writeRx(bytes: ByteArray): Boolean {
        if (!acceptWrites) return false
        writes.add(bytes)
        return true
    }

    override fun requestConnectionPriority(high: Boolean): Boolean = calls.add(if (high) "prioHigh" else "prioBalanced")
    override fun readRssi(): Boolean = calls.add("rssi")
    override fun close() {
        calls.add("close")
    }
}

class GattClientLinkTest {
    private val hello = ByteArray(16) { it.toByte() }

    private class Rig {
        val sink = RecordingSink()
        val registry = LinkRegistry(sink)
        val scheduler = ManualScheduler()
        val gatt = FakeGatt()
        val finished = mutableListOf<GattClientLink>()

        /** connectFailed reports already delivered when the client finished. */
        var failedAtFinish = -1
        var l2capRequest: Triple<Int, (Int) -> Unit, (Int?) -> Unit>? = null
        val link = GattClientLink(
            discId = 7, registry = registry, config = BleConfig.DESIGN, scheduler = scheduler,
            connectGatt = { gatt },
            l2capConnect = { psm, onClosed, onDone -> l2capRequest = Triple(psm, onClosed, onDone) },
        ) {
            finished.add(it)
            if (failedAtFinish < 0) failedAtFinish = sink.failed.size
        }
    }

    /** Connect → MTU 517 → discovery → HELLO read → WAIT_OPEN. */
    private fun Rig.toWaitOpen() {
        link.connect()
        link.gattEvents.onConnectionState(0, connected = true)
        assertEquals(517, gatt.mtuRequested)
        link.gattEvents.onMtuChanged(517, 0)
        link.gattEvents.onServicesDiscovered(0)
        link.gattEvents.onHelloRead(hello, 0)
        assertEquals(listOf("phy2m", "mtu", "discover", "resolve", "readHello"), gatt.calls)
        assertArrayEquals(hello, sink.hellos.single().second)
        assertTrue("connect timeout cancelled after HELLO", scheduler.delayed.isEmpty())
    }

    @Test
    fun gattPathEndToEnd() {
        val r = Rig()
        r.toWaitOpen()
        r.link.openGatt()
        assertTrue(r.gatt.calls.contains("subscribeTx"))
        r.link.gattEvents.onDescriptorWrite(0)
        val id = r.link.id
        assertEquals(listOf(id), r.sink.ups)
        assertEquals(517, r.link.attMtu)
        // Inbound: TX notifications become link data.
        r.link.gattEvents.onTxNotification(byteArrayOf(5, 6))
        assertArrayEquals(byteArrayOf(5, 6), r.sink.data.single().second)
        // Outbound: write without response, one in flight, then writable.
        assertEquals(FfiSendOutcome.ACCEPTED, r.link.send(byteArrayOf(1)))
        assertEquals(FfiSendOutcome.WOULD_BLOCK, r.link.send(byteArrayOf(2)))
        r.link.gattEvents.onRxWritten(0)
        assertEquals(listOf(id), r.sink.writable)
        assertEquals(FfiSendOutcome.ACCEPTED, r.link.send(byteArrayOf(2)))
        assertEquals(2, r.gatt.writes.size)
        // Connection priority for voice bursts.
        assertTrue(r.link.carries(id))
        assertTrue(r.link.setPriority(FfiLinkPriority.HIGH))
        assertTrue(r.link.setPriority(FfiLinkPriority.BALANCED))
        assertTrue(r.gatt.calls.containsAll(listOf("prioHigh", "prioBalanced")))
        // Supervision timeout: the link goes down with Timeout.
        r.link.gattEvents.onConnectionState(8, connected = false)
        assertEquals(FfiLinkDownReason.Timeout, r.sink.downs.single().second)
        assertTrue(r.link.finished)
        assertTrue(r.gatt.calls.contains("close"))
    }

    @Test
    fun refusedWriteRetriesAndSignalsWritable() {
        val r = Rig()
        r.toWaitOpen()
        r.link.openGatt()
        r.link.gattEvents.onDescriptorWrite(0)
        r.gatt.acceptWrites = false
        assertEquals(FfiSendOutcome.WOULD_BLOCK, r.link.send(byteArrayOf(1)))
        r.scheduler.runDelayed()
        assertEquals(listOf(r.link.id), r.sink.writable)
    }

    @Test
    fun gattWithoutDataPathFails() {
        val r = Rig()
        r.toWaitOpen()
        r.gatt.dataPath = false
        r.link.openGatt()
        assertEquals(7, r.sink.failed.single().first)
        assertTrue(r.sink.ups.isEmpty())
    }

    @Test
    fun l2capKeepsGattForConnectionPriority() {
        val r = Rig()
        r.toWaitOpen()
        r.link.openL2cap(0x81)
        val (psm, onClosed, onDone) = r.l2capRequest!!
        assertEquals(0x81, psm)
        onDone(42)
        assertFalse("GATT client kept while the L2CAP link lives", r.gatt.calls.contains("close"))
        assertEquals(42, r.link.l2capLinkId)
        assertTrue(r.link.carries(42))
        assertTrue(r.link.setPriority(FfiLinkPriority.HIGH))
        assertTrue(r.gatt.calls.contains("prioHigh"))
        // The L2CAP link goes down: the GATT client follows, no extra events for the core.
        onClosed(42)
        assertTrue(r.gatt.calls.contains("close"))
        assertTrue(r.link.finished)
        assertFalse(r.link.carries(42))
        assertTrue(r.sink.downs.isEmpty() && r.sink.failed.isEmpty())
    }

    @Test
    fun gattDropWhileL2capUpReportsNothing() {
        val r = Rig()
        r.toWaitOpen()
        r.link.openL2cap(0x81)
        r.l2capRequest!!.third(42)
        r.link.gattEvents.onConnectionState(8, connected = false)
        assertTrue(r.sink.downs.isEmpty() && r.sink.failed.isEmpty())
        assertFalse(r.link.setPriority(FfiLinkPriority.HIGH))
    }

    @Test
    fun l2capFailureClosesGattBeforeReportingSoTheFallbackConnectIsAccepted() {
        val r = Rig()
        r.toWaitOpen()
        r.link.openL2cap(0x81)
        val registry = r.registry
        r.l2capRequest!!.third(null)
        // The core reacts to connectFailed with a new Connect: the client must already be finished.
        assertEquals(0, r.failedAtFinish)
        assertTrue(r.link.finished)
        assertEquals(7, r.sink.failed.single().first)
        assertEquals(FfiLinkDownReason.Error(L2capLink.IO_ERROR), r.sink.failed.single().second)
        assertEquals(0, registry.count)
    }

    @Test
    fun l2capClosedBeforeResultFinishesTheClient() {
        val r = Rig()
        r.toWaitOpen()
        r.link.openL2cap(0x81)
        r.l2capRequest!!.second(42)
        r.l2capRequest!!.third(42)
        assertTrue(r.link.finished)
        assertTrue(r.gatt.calls.contains("close"))
    }

    @Test
    fun connectTimeoutAndLimitStatuses() {
        val r = Rig()
        r.link.connect()
        r.scheduler.runDelayed()
        assertEquals(FfiLinkDownReason.Timeout, r.sink.failed.single().second)
        val r2 = Rig()
        r2.link.connect()
        r2.link.gattEvents.onConnectionState(133, connected = false)
        assertEquals(FfiLinkDownReason.LimitReached, r2.sink.failed.single().second)
    }

    @Test
    fun abortWhileWaitingReportsLocalCloseOnce() {
        val r = Rig()
        r.toWaitOpen()
        r.link.abort()
        r.link.abort()
        assertTrue(r.link.finished)
        assertEquals(1, r.gatt.calls.count { it == "close" })
        assertTrue(r.sink.failed.isEmpty())
    }
}
