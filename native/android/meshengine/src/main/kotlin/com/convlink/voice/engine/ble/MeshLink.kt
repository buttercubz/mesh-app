package com.convlink.voice.engine.ble

import com.convlink.voice.core.FfiDiscovery
import com.convlink.voice.core.FfiLinkDownReason
import com.convlink.voice.core.FfiLinkInfo
import com.convlink.voice.core.FfiLinkKind
import com.convlink.voice.core.FfiRadioCommand
import com.convlink.voice.core.FfiSendOutcome

// Platform link interface (contract C9, DESIGN §9.1), Kotlin mirror of MeshPlatformKit's
// MeshLink.swift. The core sees links as data and events; this layer owns every OS object
// and never parses or frames bytes (framing is in Rust, mesh-wire). The value types are the
// UniFFI ones (FfiLinkInfo, FfiLinkDownReason, ...), so nothing is converted twice.

/** One open link (L2CAP socket, GATT client or GATT server subscription). */
interface MeshLink {
    val id: Int
    val kind: FfiLinkKind

    /** Non-blocking: takes the whole unit or nothing (`WouldBlock`, then `linkWritable`). */
    fun send(bytes: ByteArray): FfiSendOutcome

    /** Closes the OS object; `reason` is the CLOSE/REJECT byte already sent in band (or -1). */
    fun close(reason: Int)
}

/** Applies the core's radio decisions (`PlatformHooks.radio`). */
interface LinkDriver {
    fun apply(cmd: FfiRadioCommand)
}

/** Receives link and discovery events (implemented by EngineHost, through LinkRegistry). */
interface LinkSink {
    fun linkUp(link: MeshLink, info: FfiLinkInfo)
    fun linkDown(id: Int, reason: FfiLinkDownReason)
    fun linkData(id: Int, bytes: ByteArray)
    fun linkWritable(id: Int)
    fun linkMetrics(id: Int, rssi: Short?, attMtu: UShort?)
    fun discovered(d: FfiDiscovery)
    fun helloRead(discId: Int, bytes: ByteArray)
    fun connectFailed(discId: Int, reason: FfiLinkDownReason)
}

/** v2 cellular hook (DESIGN §9.8): designed, no implementation ships in v1. */
interface TunnelProvider {
    fun open(rendezvous: ByteArray, peerHint: ByteArray): MeshLink?
}
