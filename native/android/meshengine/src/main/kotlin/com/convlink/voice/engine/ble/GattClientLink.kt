package com.convlink.voice.engine.ble

import android.bluetooth.BluetoothGatt
import com.convlink.voice.core.FfiLinkDownReason
import com.convlink.voice.core.FfiLinkKind
import com.convlink.voice.core.FfiLinkPriority
import com.convlink.voice.core.FfiLinkRole
import com.convlink.voice.core.FfiSendOutcome
import com.convlink.voice.engine.LinkRegistry

/** Opens the L2CAP channel of `OpenLink{via: L2cap}`: (psm, onClosed(linkId), onDone(linkId?)). */
typealias L2capConnector = (psm: Int, onClosed: (Int) -> Unit, onDone: (Int?) -> Unit) -> Unit

/**
 * Initiator side of one discovery (DESIGN §9.3 connection sequence): `Connect{disc_id}` →
 * connectGatt (LE, 1M PHY preferred for range, [PhyPolicy]) → MTU 517 → discover `MESH_SVC` → read `HELLO` →
 * `on_hello_read`; then the core answers `OpenLink{via}` or `Abort`. Connect timeout 8 s.
 * GATT 133/257 while connecting → `LimitReached`.
 *
 * - `via: Gatt` (also the core's fallback after an L2CAP failure): subscribe `TX`
 *   (notify) and the link is up; data goes out as write-without-response to `RX` (one
 *   write in flight, `WouldBlock` → `linkWritable`) and comes in as `TX` notifications.
 * - `via: L2cap{psm}`: the L2CAP channel becomes the link and this GATT client is **kept**
 *   for the life of the socket (state `L2CAP_UP`), only so that `SetLinkParams` can
 *   request CONNECTION_PRIORITY_HIGH / BALANCED on the shared ACL; it is closed when the
 *   L2CAP link goes down. An L2CAP failure closes the GATT client first and then reports
 *   `connectFailed`, so the core's next `Connect` (GATT fallback) is never taken for a
 *   duplicate.
 */
class GattClientLink(
    val discId: Int,
    private val registry: LinkRegistry,
    private val config: BleConfig,
    private val scheduler: Scheduler,
    private val connectGatt: (GattEvents) -> GattOps?,
    private val l2capConnect: L2capConnector,
    private val onFinished: (GattClientLink) -> Unit,
) : MeshLink {
    private enum class Phase { CONNECTING, HELLO, WAIT_OPEN, SUBSCRIBING, UP, L2CAP, L2CAP_UP, DONE }

    override var id: Int = 0
        private set
    override val kind = FfiLinkKind.BLE_GATT

    private var ops: GattOps? = null
    private var phase = Phase.CONNECTING
    private var mtu = GattServer.DEFAULT_MTU
    private var inFlight = false
    private var blocked = false
    private var localClose = false

    /** The L2CAP link riding on this connection (`L2CAP_UP`), 0 otherwise. */
    @Volatile
    var l2capLinkId: Int = 0
        private set
    private val closedEarly = HashSet<Int>()
    private val timeout = Runnable { fail(LinkDownMapper.GATT_CONN_TIMEOUT) }

    /** True once the client is finished (closed); BleRadio may then replace it. */
    val finished: Boolean get() = synchronized(this) { phase == Phase.DONE }

    /** The negotiated ATT MTU (diagnostics, tests). */
    val attMtu: Int get() = synchronized(this) { mtu }

    fun connect() {
        scheduler.postDelayed(timeout, config.connectTimeoutMs)
        val o = connectGatt(events)
        if (o == null) {
            fail(LinkDownMapper.GATT_ERROR)
            return
        }
        val dead = synchronized(this) {
            if (phase != Phase.DONE) ops = o
            phase == Phase.DONE
        }
        if (dead) o.close()
    }

    /** `OpenLink{via: Gatt}`: subscribe to `TX`; the link is up when the CCCD write succeeds. */
    fun openGatt() {
        val o = synchronized(this) {
            if (phase != Phase.WAIT_OPEN) return
            phase = Phase.SUBSCRIBING
            ops
        } ?: return
        if (!o.hasDataPath || !o.enableTxNotifications()) fail(LinkDownMapper.GATT_FAILURE)
    }

    /** `OpenLink{via: L2cap{psm}}`. */
    fun openL2cap(psm: Int) {
        synchronized(this) {
            if (phase != Phase.WAIT_OPEN) return
            phase = Phase.L2CAP
        }
        l2capConnect(psm, { link -> scheduler.post { l2capClosed(link) } }, { link -> scheduler.post { l2capResult(link) } })
    }

    private fun l2capResult(link: Int?) {
        if (link == null) {
            val waiting = synchronized(this) {
                val w = phase == Phase.L2CAP
                if (w) phase = Phase.DONE
                w
            }
            if (waiting) {
                scheduler.cancel(timeout)
                closeGatt()
            }
            registry.connectFailed(discId, FfiLinkDownReason.Error(L2capLink.IO_ERROR))
            return
        }
        val keep = synchronized(this) {
            if (phase == Phase.L2CAP && !closedEarly.remove(link)) {
                phase = Phase.L2CAP_UP
                l2capLinkId = link
                true
            } else {
                false
            }
        }
        // Aborted meanwhile, or the socket already died: the GATT client has no job left.
        if (!keep) finish()
    }

    private fun l2capClosed(link: Int) {
        synchronized(this) {
            when {
                phase == Phase.L2CAP -> {
                    closedEarly.add(link)
                    return
                }
                phase != Phase.L2CAP_UP || l2capLinkId != link -> return
            }
        }
        finish()
    }

    /** `Abort{disc_id}`: drop the pending connection. */
    fun abort() {
        synchronized(this) { localClose = true }
        finish()
    }

    /** This client carries the core link [linkId] (its GATT link, or its L2CAP link's ACL). */
    fun carries(linkId: Int): Boolean = synchronized(this) {
        linkId != 0 && ((phase == Phase.UP && id == linkId) || (phase == Phase.L2CAP_UP && l2capLinkId == linkId))
    }

    /**
     * `SetLinkParams`: CONNECTION_PRIORITY_HIGH during voice bursts, BALANCED otherwise.
     * Works for GATT and L2CAP initiator links (both own a `BluetoothGatt` here).
     */
    fun setPriority(p: FfiLinkPriority): Boolean {
        val o = synchronized(this) { if (phase == Phase.UP || phase == Phase.L2CAP_UP) ops else null } ?: return false
        return o.requestConnectionPriority(p == FfiLinkPriority.HIGH)
    }

    override fun send(bytes: ByteArray): FfiSendOutcome {
        val o: GattOps
        synchronized(this) {
            if (phase != Phase.UP) return FfiSendOutcome.NO_LINK
            o = ops ?: return FfiSendOutcome.NO_LINK
            if (inFlight) {
                blocked = true
                return FfiSendOutcome.WOULD_BLOCK
            }
            inFlight = true
        }
        if (o.writeRx(bytes)) return FfiSendOutcome.ACCEPTED
        synchronized(this) {
            inFlight = false
            blocked = true
        }
        // The stack refused (busy): retry once the next write completes or soon after.
        scheduler.postDelayed({ signalWritable() }, RETRY_WRITE_MS)
        return FfiSendOutcome.WOULD_BLOCK
    }

    override fun close(reason: Int) {
        synchronized(this) { localClose = true }
        finish()
    }

    /** Requests an RSSI reading (reported through `on_link_metrics`). */
    fun pollRssi() {
        val o = synchronized(this) { if (phase == Phase.UP) ops else null } ?: return
        o.readRssi()
    }

    // MARK: internals

    private fun signalWritable() {
        val signal = synchronized(this) {
            val b = blocked && !inFlight && phase == Phase.UP
            if (b) blocked = false
            b
        }
        if (signal) registry.writable(id)
    }

    private fun fail(status: Int) {
        val prev: Phase
        synchronized(this) {
            prev = phase
            if (prev == Phase.DONE) return
            phase = Phase.DONE
        }
        scheduler.cancel(timeout)
        closeGatt()
        when (prev) {
            // The L2CAP connect reports its own outcome; an up L2CAP link lives on its own.
            Phase.L2CAP, Phase.L2CAP_UP -> Unit
            Phase.UP -> registry.down(id, LinkDownMapper.disconnect(status, localClose))
            else -> registry.connectFailed(discId,
                if (localClose) FfiLinkDownReason.LocalClose else LinkDownMapper.connectFailure(status))
        }
    }

    private fun finish() {
        val wasUp: Boolean
        synchronized(this) {
            if (phase == Phase.DONE) return
            wasUp = phase == Phase.UP
            phase = Phase.DONE
        }
        scheduler.cancel(timeout)
        closeGatt()
        if (wasUp) registry.down(id, FfiLinkDownReason.LocalClose)
    }

    private fun closeGatt() {
        val o = synchronized(this) { ops.also { ops = null } }
        o?.close()
        onFinished(this)
    }

    private val events = object : GattEvents {
        override fun onConnectionState(status: Int, connected: Boolean) {
            val o = synchronized(this@GattClientLink) { ops }
            if (connected && status == BluetoothGatt.GATT_SUCCESS) {
                if (o == null) return
                o.setPreferredPhy1M()
                if (!o.requestMtu(config.preferredAttMtu) && !o.discoverServices()) fail(LinkDownMapper.GATT_FAILURE)
            } else {
                fail(if (status == BluetoothGatt.GATT_SUCCESS && !connected) LinkDownMapper.GATT_CONN_TERMINATE_PEER_USER else status)
            }
        }

        override fun onMtuChanged(mtu: Int, status: Int) {
            val (o, up) = synchronized(this@GattClientLink) {
                if (status == BluetoothGatt.GATT_SUCCESS) this@GattClientLink.mtu = mtu
                ops to (phase == Phase.UP)
            }
            if (up) {
                registry.metrics(id, null, attMtu.toUShort())
            } else if (synchronized(this@GattClientLink) { phase == Phase.CONNECTING }) {
                if (o?.discoverServices() != true) fail(LinkDownMapper.GATT_FAILURE)
            }
        }

        override fun onServicesDiscovered(status: Int) {
            val o = synchronized(this@GattClientLink) { if (phase == Phase.CONNECTING) ops else null } ?: return
            if (status != BluetoothGatt.GATT_SUCCESS || !o.resolveService()) return fail(LinkDownMapper.GATT_FAILURE)
            synchronized(this@GattClientLink) { phase = Phase.HELLO }
            if (!o.readHello()) fail(LinkDownMapper.GATT_FAILURE)
        }

        override fun onHelloRead(value: ByteArray, status: Int) {
            if (status != BluetoothGatt.GATT_SUCCESS) return fail(status)
            synchronized(this@GattClientLink) {
                if (phase != Phase.HELLO) return
                phase = Phase.WAIT_OPEN
            }
            scheduler.cancel(timeout)
            registry.helloRead(discId, value)
        }

        override fun onDescriptorWrite(status: Int) {
            if (status != BluetoothGatt.GATT_SUCCESS) return fail(status)
            synchronized(this@GattClientLink) {
                if (phase != Phase.SUBSCRIBING) return
                phase = Phase.UP
                id = registry.allocateId()
            }
            registry.up(this@GattClientLink, gattInfo(FfiLinkRole.INITIATOR, attMtu, config, discId))
        }

        override fun onTxNotification(value: ByteArray) {
            if (synchronized(this@GattClientLink) { phase == Phase.UP }) registry.data(id, value)
        }

        override fun onRxWritten(status: Int) {
            synchronized(this@GattClientLink) { inFlight = false }
            signalWritable()
        }

        override fun onRssi(rssi: Int, status: Int) {
            if (status == BluetoothGatt.GATT_SUCCESS && synchronized(this@GattClientLink) { phase == Phase.UP }) {
                registry.metrics(id, DiscoveryParser.rssi(rssi), attMtu.toUShort())
            }
        }
    }

    /** The events sink (the JVM tests drive it directly). */
    internal val gattEvents: GattEvents get() = events

    companion object {
        /** Retry delay after the stack refused a write without response. */
        const val RETRY_WRITE_MS = 20L
    }
}
