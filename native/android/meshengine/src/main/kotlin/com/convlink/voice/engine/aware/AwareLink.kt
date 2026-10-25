package com.convlink.voice.engine.aware

import com.convlink.voice.core.FfiLinkInfo
import com.convlink.voice.core.FfiLinkKind
import com.convlink.voice.core.FfiLinkRole
import com.convlink.voice.core.FfiLinkUnit
import com.convlink.voice.core.FfiPeerPlatform
import com.convlink.voice.core.FfiSendOutcome
import com.convlink.voice.engine.ble.MeshLink

/**
 * One Wi-Fi Aware link: kind `WifiAware`, unit `Datagram` (one LinkFrame per UDP datagram,
 * no extra framing; the core frames and seals, DESIGN §4.2, §9.1). UDP never blocks, so
 * `send` never answers `WouldBlock` and no `Writable` follows.
 */
class AwareLink(
    override val id: Int,
    private val io: DatagramIo,
    private val onClose: (AwareLink) -> Unit,
) : MeshLink {
    override val kind: FfiLinkKind = FfiLinkKind.WIFI_AWARE

    @Volatile
    private var closed = false

    override fun send(bytes: ByteArray): FfiSendOutcome = when {
        closed -> FfiSendOutcome.NO_LINK
        bytes.size > MAX_FRAME -> FfiSendOutcome.NO_LINK
        io.send(bytes) -> FfiSendOutcome.ACCEPTED
        else -> FfiSendOutcome.NO_LINK
    }

    /** Closed by the core (`RadioCommand::Close`); `reason` was already sent in band. */
    override fun close(reason: Int) {
        if (closed) return
        closed = true
        onClose(this)
    }

    companion object {
        /** `MAX_DATAGRAM_FRAME` of mesh-wire (§4.2): fits one IPv6 UDP datagram. */
        const val MAX_FRAME = 1200

        fun info(role: AwareRole) = FfiLinkInfo(
            kind = FfiLinkKind.WIFI_AWARE,
            role = if (role == AwareRole.SUBSCRIBER) FfiLinkRole.INITIATOR else FfiLinkRole.RESPONDER,
            maxFrame = MAX_FRAME.toUShort(),
            unit = FfiLinkUnit.DATAGRAM,
            peerPlatformHint = FfiPeerPlatform.ANDROID,
            attMtu = null,
            discId = null,
        )
    }
}
