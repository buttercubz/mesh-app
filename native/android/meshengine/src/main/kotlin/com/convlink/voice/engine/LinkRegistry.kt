package com.convlink.voice.engine

import com.convlink.voice.core.FfiDiscovery
import com.convlink.voice.core.FfiLinkDownReason
import com.convlink.voice.core.FfiLinkInfo
import com.convlink.voice.core.FfiLinkKind
import com.convlink.voice.core.FfiSendOutcome
import com.convlink.voice.engine.ble.LinkSink
import com.convlink.voice.engine.ble.MeshLink

/**
 * Assigns link ids, keeps the live links and fans their events into the [LinkSink]
 * (DESIGN §9.1). Every driver reports through one registry, so ids are unique across
 * transports and `linkDown` is delivered exactly once per link. Thread-safe; the sink is
 * called outside the lock.
 */
class LinkRegistry(sink: LinkSink? = null) {
    private val lock = Any()
    private var nextId = 1
    private val links = HashMap<Int, MeshLink>()
    private val infos = HashMap<Int, FfiLinkInfo>()

    @Volatile
    var sink: LinkSink? = sink

    /** A fresh id: never 0 and never one still in use (wraps after 2^32 - 1 as unsigned). */
    fun allocateId(): Int = synchronized(lock) {
        var id = nextId
        while (id == 0 || links.containsKey(id)) id++
        nextId = id + 1
        id
    }

    /** Registers an open link and reports `linkUp`. */
    fun up(link: MeshLink, info: FfiLinkInfo) {
        synchronized(lock) {
            links[link.id] = link
            infos[link.id] = info
        }
        sink?.linkUp(link, info)
    }

    /** Removes the link and reports `linkDown` once; returns false when already gone. */
    fun down(id: Int, reason: FfiLinkDownReason): Boolean {
        val removed = synchronized(lock) {
            infos.remove(id)
            links.remove(id) != null
        }
        if (removed) sink?.linkDown(id, reason)
        return removed
    }

    /** Inbound bytes; dropped when the link is no longer registered. */
    fun data(id: Int, bytes: ByteArray) {
        if (contains(id)) sink?.linkData(id, bytes)
    }

    fun writable(id: Int) {
        if (contains(id)) sink?.linkWritable(id)
    }

    fun metrics(id: Int, rssi: Short?, attMtu: UShort?) {
        if (contains(id)) sink?.linkMetrics(id, rssi, attMtu)
    }

    fun discovered(d: FfiDiscovery) {
        sink?.discovered(d)
    }

    fun helloRead(discId: Int, bytes: ByteArray) {
        sink?.helloRead(discId, bytes)
    }

    fun connectFailed(discId: Int, reason: FfiLinkDownReason) {
        sink?.connectFailed(discId, reason)
    }

    fun contains(id: Int): Boolean = synchronized(lock) { links.containsKey(id) }

    fun link(id: Int): MeshLink? = synchronized(lock) { links[id] }

    fun info(id: Int): FfiLinkInfo? = synchronized(lock) { infos[id] }

    val count: Int get() = synchronized(lock) { links.size }

    fun ids(kind: FfiLinkKind? = null): List<Int> = synchronized(lock) {
        infos.filter { kind == null || it.value.kind == kind }.keys.sorted()
    }

    /** Non-blocking send by id (`PlatformHooks.link_send`). */
    fun send(id: Int, bytes: ByteArray): FfiSendOutcome =
        link(id)?.send(bytes) ?: FfiSendOutcome.NO_LINK

    /**
     * Reports every link of [kinds] down with [reason] (Bluetooth off) and returns their
     * ids. The links themselves are closed by their driver.
     */
    fun dropAll(kinds: Set<FfiLinkKind>, reason: FfiLinkDownReason): List<Int> {
        val ids = synchronized(lock) { infos.filter { it.value.kind in kinds }.keys.sorted() }
        return ids.filter { down(it, reason) }
    }
}
