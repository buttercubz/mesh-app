package com.convlink.voice.engine.aware

import com.convlink.voice.core.FfiLinkDownReason
import com.convlink.voice.core.FfiRadioCommand
import com.convlink.voice.engine.LinkRegistry

/**
 * Android↔Android Wi-Fi Aware upgrade driver (DESIGN §9.6-§9.7, WP-24, beta).
 *
 * The core runs the UPGRADE exchange over BLE and mirrors it with
 * `RadioCommand::AwareUpgrade{peer, op}`: 1 offer sent, 2 accepted (build the data path; the
 * PMK comes from `takePmk`), 3 rejected, 4 ready, 5 retire. This driver:
 *
 * 1. attaches to Wi-Fi Aware on the first op 2 and releases it when no peer is left;
 * 2. the offerer (op 1 then 2) subscribes to "cvic", the acceptor publishes it;
 * 3. follow-up handshake: the subscriber sends `REQ{tag}` to every discovered publisher,
 *    the publisher with the matching pending peer answers `ACK{tag}` (tag = hash of the
 *    shared PMK, so no node ids go on air);
 * 4. both sides request the NAN data path with `setPmk(pmk)`; the adapter binds a UDP
 *    socket to the Aware Network;
 * 5. the initiator probes until the responder echoes, then each side reports the link up
 *    into the core (kind WifiAware, unit Datagram); the core runs a fresh Noise handshake +
 *    IDPROOF on it and adds it to the neighbor's bundle (BLE stays as fallback).
 *
 * Threading: every entry point posts onto [post] (one serial thread); OS callbacks too.
 */
class AwareDriver(
    private val registry: LinkRegistry,
    private val os: AwareOs,
    private val takePmk: (peer: Int) -> ByteArray?,
    private val post: (Runnable) -> Unit,
    private val schedule: (delayMs: Long, Runnable) -> Unit,
    private val now: () -> Long,
    private val log: (String) -> Unit = {},
) {
    private enum class Attach { NONE, PENDING, ATTACHED }

    private class Peer(
        val node: Int,
        val role: AwareRole,
        val pmk: ByteArray,
        val tag: ByteArray,
        val key: Int,
        val deadline: Long,
    ) {
        var handle: Any? = null
        var io: DatagramIo? = null
        var link: AwareLink? = null
        var probes = 0
    }

    private var enabled = false
    private var attach = Attach.NONE
    private var nextKey = 1
    private var ticking = false
    private val offered = HashSet<Int>()
    private val peers = LinkedHashMap<Int, Peer>()
    private val discovered = ArrayList<Any>()

    private val listener = object : AwareOsListener {
        override fun onAttached() = post { attached() }
        override fun onAttachFailed() = post { attachFailed() }
        override fun onUnavailable() = post { dropAll(FfiLinkDownReason.RadioOff, "aware unavailable") }
        override fun onServiceDiscovered(handle: Any) = post { serviceDiscovered(handle) }
        override fun onMessage(handle: Any, fromPublisher: Boolean, bytes: ByteArray) =
            post { message(handle, fromPublisher, bytes) }
        override fun onDataPathUp(key: Int, io: DatagramIo) = post { dataPathUp(key, io) }
        override fun onDataPathLost(key: Int) = post { dataPathLost(key) }
    }

    /** The beta toggle and the OS availability together (RadioStateMonitor + Settings). */
    fun setEnabled(on: Boolean) = post {
        if (enabled == on) return@post
        enabled = on
        if (!on) dropAll(FfiLinkDownReason.RadioOff, "aware disabled")
    }

    /** `RadioCommand::AwareUpgrade` from the core. */
    fun apply(cmd: FfiRadioCommand.AwareUpgrade) {
        val peer = cmd.peer.toInt()
        val op = cmd.op.toInt()
        // Take the PMK on the engine thread right away, so a later op cannot race it.
        val pmk = if (op == OP_ACCEPTED) takePmk(peer) else null
        post { upgrade(peer, op, pmk) }
    }

    /** Session stop: closes every Aware link and releases the OS sessions. */
    fun stop() = post { dropAll(FfiLinkDownReason.LocalClose, "stop") }

    /** Peers with a link reported up (diagnostics, tests). */
    val linkCount: Int get() = peers.values.count { it.link != null }

    // MARK: core commands

    private fun upgrade(peer: Int, op: Int, pmk: ByteArray?) {
        when (op) {
            OP_OFFERED -> offered += peer
            OP_ACCEPTED -> accepted(peer, pmk)
            OP_REJECTED -> {
                offered -= peer
                remove(peer, FfiLinkDownReason.LocalClose)
            }
            OP_READY -> log("aware: peer $peer ready")
            OP_RETIRE -> {
                offered -= peer
                remove(peer, FfiLinkDownReason.LocalClose)
            }
            else -> log("aware: unknown op $op")
        }
    }

    private fun accepted(peer: Int, pmk: ByteArray?) {
        val role = if (offered.remove(peer)) AwareRole.SUBSCRIBER else AwareRole.PUBLISHER
        if (!enabled || !os.available()) {
            log("aware: op 2 for $peer ignored (enabled=$enabled)")
            return
        }
        if (pmk == null || pmk.size != 32) {
            // No PMK, no data path: an open NAN data path is never built.
            log("aware: no PMK for $peer, upgrade not built")
            return
        }
        remove(peer, FfiLinkDownReason.LocalClose)
        val p = Peer(peer, role, pmk, AwareCodec.tag(pmk), nextKey++, now() + SETUP_TIMEOUT_MS)
        peers[peer] = p
        log("aware: building ${role.name.lowercase()} path to $peer")
        when (attach) {
            Attach.NONE -> {
                attach = Attach.PENDING
                os.attach(listener)
            }
            Attach.PENDING -> {}
            Attach.ATTACHED -> startSessions()
        }
        ensureTick()
    }

    // MARK: OS events

    private fun attached() {
        if (attach != Attach.PENDING) return
        attach = Attach.ATTACHED
        startSessions()
    }

    private fun attachFailed() {
        log("aware: attach failed")
        attach = Attach.NONE
        peers.keys.toList().forEach { remove(it, FfiLinkDownReason.Error(-1)) }
    }

    private fun startSessions() {
        if (peers.values.any { it.role == AwareRole.PUBLISHER && it.handle == null }) os.publish()
        if (peers.values.any { it.role == AwareRole.SUBSCRIBER && it.handle == null }) {
            os.subscribe()
            sendRequests()
        }
    }

    private fun serviceDiscovered(handle: Any) {
        if (discovered.none { it == handle }) discovered += handle
        sendRequests()
    }

    private fun sendRequests() {
        for (p in peers.values) {
            if (p.role != AwareRole.SUBSCRIBER || p.handle != null) continue
            for (h in discovered) os.sendMessage(h, false, AwareCodec.req(p.tag))
        }
    }

    private fun message(handle: Any, fromPublisher: Boolean, bytes: ByteArray) {
        if (fromPublisher) {
            // I publish: a subscriber asks for one of my pending peers.
            val tag = AwareCodec.parse(bytes, ack = false) ?: return
            val p = peers.values.firstOrNull { it.role == AwareRole.PUBLISHER && it.tag.contentEquals(tag) } ?: return
            os.sendMessage(handle, true, AwareCodec.ack(tag))
            if (p.handle == null) {
                p.handle = handle
                requestPath(p)
            }
        } else {
            val tag = AwareCodec.parse(bytes, ack = true) ?: return
            val p = peers.values.firstOrNull { it.role == AwareRole.SUBSCRIBER && it.tag.contentEquals(tag) } ?: return
            if (p.handle == null) {
                p.handle = handle
                requestPath(p)
            }
        }
    }

    private fun requestPath(p: Peer) {
        if (!os.canOpenDataPath()) {
            log("aware: no data path available (getAvailableAwareResources)")
            remove(p.node, FfiLinkDownReason.LimitReached)
            return
        }
        os.requestDataPath(p.key, p.role, p.handle ?: return, p.pmk)
    }

    private fun dataPathUp(key: Int, io: DatagramIo) {
        val p = peers.values.firstOrNull { it.key == key }
        if (p == null || p.io != null) {
            io.close()
            return
        }
        p.io = io
        io.start { bytes -> post { datagram(key, bytes) } }
        if (p.role == AwareRole.SUBSCRIBER) probe(key)
    }

    private fun probe(key: Int) {
        val p = peers.values.firstOrNull { it.key == key } ?: return
        if (p.link != null || p.probes >= MAX_PROBES) return
        p.probes++
        p.io?.send(AwareCodec.PROBE)
        schedule(PROBE_INTERVAL_MS) { post { probe(key) } }
    }

    private fun datagram(key: Int, bytes: ByteArray) {
        val p = peers.values.firstOrNull { it.key == key } ?: return
        if (AwareCodec.isProbe(bytes)) {
            if (p.role == AwareRole.PUBLISHER) p.io?.send(AwareCodec.PROBE)
            if (p.link == null) linkUp(p)
            return
        }
        val link = p.link ?: return
        registry.data(link.id, bytes)
    }

    private fun linkUp(p: Peer) {
        val io = p.io ?: return
        val link = AwareLink(registry.allocateId(), io) { l -> post { closedByCore(l) } }
        p.link = link
        log("aware: link ${link.id} up with ${p.node}")
        registry.up(link, AwareLink.info(p.role))
    }

    private fun dataPathLost(key: Int) {
        val p = peers.values.firstOrNull { it.key == key } ?: return
        log("aware: data path to ${p.node} lost")
        remove(p.node, FfiLinkDownReason.Timeout)
    }

    private fun closedByCore(link: AwareLink) {
        val p = peers.values.firstOrNull { it.link === link }
        if (p != null) remove(p.node, FfiLinkDownReason.LocalClose) else registry.down(link.id, FfiLinkDownReason.LocalClose)
    }

    // MARK: teardown

    private fun remove(peer: Int, reason: FfiLinkDownReason) {
        val p = peers.remove(peer) ?: return
        p.io?.close()
        if (p.handle != null) os.releaseDataPath(p.key)
        p.link?.let { registry.down(it.id, reason) }
        if (peers.isEmpty()) releaseOs()
    }

    private fun dropAll(reason: FfiLinkDownReason, why: String) {
        if (peers.isNotEmpty() || attach != Attach.NONE) log("aware: drop all ($why)")
        offered.clear()
        peers.keys.toList().forEach { remove(it, reason) }
        releaseOs()
    }

    private fun releaseOs() {
        if (attach == Attach.NONE) return
        attach = Attach.NONE
        discovered.clear()
        os.release()
    }

    private fun ensureTick() {
        if (ticking) return
        ticking = true
        schedule(TICK_MS) { post { tick() } }
    }

    /** Re-sends REQs (follow-ups may be lost) and abandons setups past their deadline. */
    private fun tick() {
        ticking = false
        val t = now()
        peers.values.filter { it.link == null && t >= it.deadline }.map { it.node }.forEach {
            log("aware: setup with $it timed out")
            remove(it, FfiLinkDownReason.Timeout)
        }
        if (attach == Attach.ATTACHED) sendRequests()
        if (peers.values.any { it.link == null }) ensureTick()
    }

    companion object {
        const val OP_OFFERED = 1
        const val OP_ACCEPTED = 2
        const val OP_REJECTED = 3
        const val OP_READY = 4
        const val OP_RETIRE = 5
        const val SETUP_TIMEOUT_MS = 30_000L
        const val TICK_MS = 1_000L
        const val PROBE_INTERVAL_MS = 200L
        const val MAX_PROBES = 100
    }
}
