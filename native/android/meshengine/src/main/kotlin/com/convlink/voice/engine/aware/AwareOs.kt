package com.convlink.voice.engine.aware

import java.security.MessageDigest

// Wi-Fi Aware (NAN) seam for the Android↔Android upgrade (DESIGN §9.6, WP-24 beta).
// [AwareDriver] holds the state machine and talks to the OS only through [AwareOs], so the
// JVM tests drive it with fakes; [AndroidAwareOs] is the WifiAwareManager implementation.

/** Which side of the NAN data path this phone is for one peer. */
enum class AwareRole {
    /** The UPGRADE offerer (lower `node_id`): subscribes, data-path initiator, Noise initiator. */
    SUBSCRIBER,

    /** The UPGRADE acceptor: publishes, data-path responder, Noise responder. */
    PUBLISHER,
}

/** An open datagram path over the Aware network (UDP over IPv6 link-local). */
interface DatagramIo {
    /** Non-blocking best effort; false when the socket is gone or the peer is unknown yet. */
    fun send(bytes: ByteArray): Boolean

    /** Starts delivering every inbound datagram to [onData] (on any thread). */
    fun start(onData: (ByteArray) -> Unit)

    fun close()
}

/** OS events the adapter reports back (posted to the driver thread by the driver). */
interface AwareOsListener {
    fun onAttached()
    fun onAttachFailed()

    /** The Wi-Fi Aware service went away (ACTION_WIFI_AWARE_STATE_CHANGED, hotspot on, ...). */
    fun onUnavailable()

    /** Subscriber side: a "cvic" publisher was found (opaque `handle`, a PeerHandle). */
    fun onServiceDiscovered(handle: Any)

    /** A follow-up message from `handle` on the publish or subscribe session. */
    fun onMessage(handle: Any, fromPublisher: Boolean, bytes: ByteArray)

    /** The data path for `key` is up with a bound socket. */
    fun onDataPathUp(key: Int, io: DatagramIo)

    /** The data path for `key` failed or was lost. */
    fun onDataPathLost(key: Int)
}

/** The Android Wi-Fi Aware API, reduced to what the driver needs. */
interface AwareOs {
    /** FEATURE_WIFI_AWARE present and `WifiAwareManager.isAvailable()`. */
    fun available(): Boolean

    /** `getAvailableAwareResources().availableDataPathsCount > 0` (true below API 31 checks). */
    fun canOpenDataPath(): Boolean

    fun attach(listener: AwareOsListener)

    /** Starts the unsolicited "cvic" publish session (idempotent). */
    fun publish()

    /** Starts the passive "cvic" subscribe session (idempotent). */
    fun subscribe()

    fun sendMessage(handle: Any, fromPublisher: Boolean, bytes: ByteArray)

    /**
     * Requests the NAN data path to `handle` secured with `pmk` (`setPmk`), then binds a UDP
     * socket to that Network. `key` identifies the request in [AwareOsListener] callbacks.
     */
    fun requestDataPath(key: Int, role: AwareRole, handle: Any, pmk: ByteArray)

    fun releaseDataPath(key: Int)

    /** Closes every session and the attach session (stop, radio off). */
    fun release()
}

/** Follow-up messages and probes (fixed, versioned, no names, no node ids). */
object AwareCodec {
    const val SERVICE = "cvic"
    private val REQ = byteArrayOf('C'.code.toByte(), 'V'.code.toByte(), 1, 'Q'.code.toByte())
    private val ACK = byteArrayOf('C'.code.toByte(), 'V'.code.toByte(), 1, 'A'.code.toByte())

    /** 4-byte UDP probe: punches the path before the core's first frame (never delivered). */
    val PROBE = byteArrayOf('C'.code.toByte(), 'V'.code.toByte(), 'A'.code.toByte(), 'P'.code.toByte())

    /**
     * Rendezvous tag of one upgrade: SHA-256("cvic/v1/aware-tag" ‖ pmk)[0..8]. Both ends
     * hold the same PMK, so the tag matches only the intended peer and reveals nothing
     * about the key.
     */
    fun tag(pmk: ByteArray): ByteArray {
        val md = MessageDigest.getInstance("SHA-256")
        md.update("cvic/v1/aware-tag".toByteArray(Charsets.US_ASCII))
        md.update(pmk)
        return md.digest().copyOf(8)
    }

    fun req(tag: ByteArray) = REQ + tag

    fun ack(tag: ByteArray) = ACK + tag

    /** The tag of a REQ (`ack = false`) or ACK (`ack = true`) message, or null. */
    fun parse(bytes: ByteArray, ack: Boolean): ByteArray? {
        val head = if (ack) ACK else REQ
        if (bytes.size != head.size + 8) return null
        if (!bytes.copyOf(head.size).contentEquals(head)) return null
        return bytes.copyOfRange(head.size, bytes.size)
    }

    fun isProbe(bytes: ByteArray) = bytes.contentEquals(PROBE)
}
