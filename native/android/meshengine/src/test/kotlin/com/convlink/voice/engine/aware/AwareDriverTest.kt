package com.convlink.voice.engine.aware

import com.convlink.voice.core.FfiLinkDownReason
import com.convlink.voice.core.FfiLinkKind
import com.convlink.voice.core.FfiLinkRole
import com.convlink.voice.core.FfiLinkUnit
import com.convlink.voice.core.FfiRadioCommand
import com.convlink.voice.core.FfiSendOutcome
import com.convlink.voice.engine.LinkRegistry
import com.convlink.voice.engine.ble.RecordingSink
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class FakeIo : DatagramIo {
    val sent = ArrayList<ByteArray>()
    var onData: ((ByteArray) -> Unit)? = null
    var closed = false

    override fun send(bytes: ByteArray): Boolean {
        if (closed) return false
        sent += bytes
        return true
    }

    override fun start(onData: (ByteArray) -> Unit) {
        this.onData = onData
    }

    override fun close() {
        closed = true
    }

    fun deliver(bytes: ByteArray) = onData!!.invoke(bytes)
}

class FakeAwareOs : AwareOs {
    var available = true
    var resources = true
    var listener: AwareOsListener? = null
    var attaches = 0
    var publishes = 0
    var subscribes = 0
    var releases = 0
    val messages = ArrayList<Triple<Any, Boolean, ByteArray>>()
    val requests = ArrayList<Triple<Int, AwareRole, ByteArray>>()
    val released = ArrayList<Int>()

    override fun available() = available
    override fun canOpenDataPath() = resources
    override fun attach(listener: AwareOsListener) {
        attaches++
        this.listener = listener
    }
    override fun publish() {
        publishes++
    }
    override fun subscribe() {
        subscribes++
    }
    override fun sendMessage(handle: Any, fromPublisher: Boolean, bytes: ByteArray) {
        messages += Triple(handle, fromPublisher, bytes)
    }
    override fun requestDataPath(key: Int, role: AwareRole, handle: Any, pmk: ByteArray) {
        requests += Triple(key, role, pmk)
    }
    override fun releaseDataPath(key: Int) {
        released += key
    }
    override fun release() {
        releases++
    }
}

class AwareDriverTest {
    private val pmk = ByteArray(32) { it.toByte() }
    private val sink = RecordingSink()
    private val registry = LinkRegistry(sink)
    private val os = FakeAwareOs()
    private var clock = 0L
    private val timers = ArrayList<Pair<Long, Runnable>>()
    private val pmks = HashMap<Int, ByteArray>()
    private val driver = AwareDriver(
        registry = registry,
        os = os,
        takePmk = { pmks.remove(it) },
        post = { it.run() },
        schedule = { ms, r -> timers += (clock + ms) to r },
        now = { clock },
    )

    private fun cmd(peer: Int, op: Int) = FfiRadioCommand.AwareUpgrade(peer.toUShort(), op.toUByte())

    private fun advance(ms: Long) {
        clock += ms
        while (true) {
            val due = timers.filter { it.first <= clock }
            if (due.isEmpty()) return
            timers.removeAll(due.toSet())
            due.forEach { it.second.run() }
        }
    }

    private fun accept(peer: Int, offered: Boolean) {
        driver.setEnabled(true)
        pmks[peer] = pmk
        if (offered) driver.apply(cmd(peer, AwareDriver.OP_OFFERED))
        driver.apply(cmd(peer, AwareDriver.OP_ACCEPTED))
    }

    /** Offerer side up to a running link; returns the io. */
    private fun subscriberLink(peer: Int = 7): FakeIo {
        accept(peer, offered = true)
        os.listener!!.onAttached()
        os.listener!!.onServiceDiscovered("pub")
        os.listener!!.onMessage("pub", false, AwareCodec.ack(AwareCodec.tag(pmk)))
        val io = FakeIo()
        os.listener!!.onDataPathUp(os.requests.last().first, io)
        io.deliver(AwareCodec.PROBE)
        return io
    }

    @Test
    fun offererSubscribesHandshakesAndBringsTheLinkUpAsInitiator() {
        accept(7, offered = true)
        assertEquals(1, os.attaches)
        os.listener!!.onAttached()
        assertEquals(1, os.subscribes)
        assertEquals(0, os.publishes)
        os.listener!!.onServiceDiscovered("pub")
        val req = os.messages.single()
        assertEquals("pub", req.first)
        assertFalse(req.second)
        assertArrayEquals(AwareCodec.tag(pmk), AwareCodec.parse(req.third, ack = false))

        os.listener!!.onMessage("pub", false, AwareCodec.ack(AwareCodec.tag(pmk)))
        val (key, role, usedPmk) = os.requests.single()
        assertEquals(AwareRole.SUBSCRIBER, role)
        assertArrayEquals(pmk, usedPmk)

        val io = FakeIo()
        os.listener!!.onDataPathUp(key, io)
        assertArrayEquals(AwareCodec.PROBE, io.sent.single())
        assertTrue(sink.ups.isEmpty())
        advance(AwareDriver.PROBE_INTERVAL_MS)
        assertEquals(2, io.sent.size)

        io.deliver(AwareCodec.PROBE)
        val id = sink.ups.single()
        val info = registry.info(id)!!
        assertEquals(FfiLinkKind.WIFI_AWARE, info.kind)
        assertEquals(FfiLinkUnit.DATAGRAM, info.unit)
        assertEquals(FfiLinkRole.INITIATOR, info.role)
        assertEquals(1, driver.linkCount)

        // Frames go both ways unchanged; probes never reach the core.
        io.deliver(byteArrayOf(1, 2, 3))
        io.deliver(AwareCodec.PROBE)
        assertEquals(1, sink.data.size)
        assertArrayEquals(byteArrayOf(1, 2, 3), sink.data.single().second)
        assertEquals(FfiSendOutcome.ACCEPTED, registry.send(id, byteArrayOf(9)))
        assertArrayEquals(byteArrayOf(9), io.sent.last())
        assertEquals(FfiSendOutcome.NO_LINK, registry.send(id, ByteArray(AwareLink.MAX_FRAME + 1)))
    }

    @Test
    fun acceptorPublishesMatchesTheTagAndAnswersAsResponder() {
        accept(9, offered = false)
        os.listener!!.onAttached()
        assertEquals(1, os.publishes)
        assertEquals(0, os.subscribes)
        // A REQ for someone else's upgrade is ignored.
        os.listener!!.onMessage("sub", true, AwareCodec.req(AwareCodec.tag(ByteArray(32))))
        assertTrue(os.messages.isEmpty())
        assertTrue(os.requests.isEmpty())

        os.listener!!.onMessage("sub", true, AwareCodec.req(AwareCodec.tag(pmk)))
        val ack = os.messages.single()
        assertTrue(ack.second)
        assertArrayEquals(AwareCodec.tag(pmk), AwareCodec.parse(ack.third, ack = true))
        val (key, role, _) = os.requests.single()
        assertEquals(AwareRole.PUBLISHER, role)

        val io = FakeIo()
        os.listener!!.onDataPathUp(key, io)
        assertTrue(io.sent.isEmpty()) // the responder waits for the initiator's probe
        io.deliver(AwareCodec.PROBE)
        assertArrayEquals(AwareCodec.PROBE, io.sent.single()) // echo
        assertEquals(FfiLinkRole.RESPONDER, registry.info(sink.ups.single())!!.role)
    }

    @Test
    fun noPmkOrDisabledBuildsNothing() {
        driver.apply(cmd(3, AwareDriver.OP_ACCEPTED)) // disabled
        driver.setEnabled(true)
        driver.apply(cmd(3, AwareDriver.OP_ACCEPTED)) // enabled, no PMK
        os.available = false
        pmks[3] = pmk
        driver.apply(cmd(3, AwareDriver.OP_ACCEPTED)) // OS unavailable
        assertEquals(0, os.attaches)
    }

    @Test
    fun retireClosesTheLinkAndReleasesAware() {
        val io = subscriberLink(7)
        val id = sink.ups.single()
        driver.apply(cmd(7, AwareDriver.OP_RETIRE))
        assertEquals(listOf(id to FfiLinkDownReason.LocalClose), sink.downs.toList())
        assertTrue(io.closed)
        assertEquals(1, os.released.size)
        assertEquals(1, os.releases)
        assertEquals(0, driver.linkCount)
    }

    @Test
    fun awareUnavailableDropsEveryLinkAsRadioOff() {
        subscriberLink(7)
        os.listener!!.onUnavailable()
        assertEquals(FfiLinkDownReason.RadioOff, sink.downs.single().second)
        assertEquals(1, os.releases)
    }

    @Test
    fun betaToggleOffDropsLinks() {
        subscriberLink(7)
        driver.setEnabled(false)
        assertEquals(FfiLinkDownReason.RadioOff, sink.downs.single().second)
    }

    @Test
    fun coreCloseAndDataPathLoss() {
        subscriberLink(7)
        val id = sink.ups.single()
        registry.link(id)!!.close(-1)
        assertEquals(listOf(id to FfiLinkDownReason.LocalClose), sink.downs.toList())

        sink.downs.clear()
        subscriberLink(8)
        val id2 = sink.ups.last()
        os.listener!!.onDataPathLost(os.requests.last().first)
        assertEquals(listOf(id2 to FfiLinkDownReason.Timeout), sink.downs.toList())
    }

    @Test
    fun setupTimesOutAndResendsRequests() {
        accept(7, offered = true)
        os.listener!!.onAttached()
        os.listener!!.onServiceDiscovered("pub")
        assertEquals(1, os.messages.size)
        advance(AwareDriver.TICK_MS)
        assertEquals(2, os.messages.size) // REQ re-sent: follow-ups may be lost
        advance(AwareDriver.SETUP_TIMEOUT_MS)
        assertEquals(1, os.releases)
        assertTrue(sink.ups.isEmpty())
        assertTrue(sink.downs.isEmpty())
    }

    @Test
    fun noDataPathResourcesAbandonsTheUpgrade() {
        os.resources = false
        accept(7, offered = true)
        os.listener!!.onAttached()
        os.listener!!.onServiceDiscovered("pub")
        os.listener!!.onMessage("pub", false, AwareCodec.ack(AwareCodec.tag(pmk)))
        assertTrue(os.requests.isEmpty())
        assertEquals(1, os.releases)
    }

    @Test
    fun rejectedOfferForgetsTheRole() {
        driver.setEnabled(true)
        driver.apply(cmd(5, AwareDriver.OP_OFFERED))
        driver.apply(cmd(5, AwareDriver.OP_REJECTED))
        pmks[5] = pmk
        driver.apply(cmd(5, AwareDriver.OP_ACCEPTED)) // a later accept of their offer
        os.listener!!.onAttached()
        assertEquals(1, os.publishes)
        assertEquals(0, os.subscribes)
    }

    @Test
    fun codecTagIsStableAndKeyed() {
        assertArrayEquals(AwareCodec.tag(pmk), AwareCodec.tag(pmk.copyOf()))
        assertFalse(AwareCodec.tag(pmk).contentEquals(AwareCodec.tag(ByteArray(32))))
        assertEquals(8, AwareCodec.tag(pmk).size)
        assertNull(AwareCodec.parse(AwareCodec.req(AwareCodec.tag(pmk)), ack = true))
        assertNotNull(AwareCodec.parse(AwareCodec.ack(AwareCodec.tag(pmk)), ack = true))
        assertNull(AwareCodec.parse(byteArrayOf(1, 2), ack = false))
        assertTrue(AwareCodec.isProbe(AwareCodec.PROBE.copyOf()))
    }
}
