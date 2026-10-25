package com.convlink.voice.engine.ble

import android.annotation.SuppressLint
import android.bluetooth.BluetoothDevice
import android.bluetooth.BluetoothGatt
import android.bluetooth.BluetoothGattCharacteristic
import android.bluetooth.BluetoothGattDescriptor
import android.bluetooth.BluetoothGattServer
import android.bluetooth.BluetoothGattServerCallback
import android.bluetooth.BluetoothGattService
import android.bluetooth.BluetoothManager
import android.bluetooth.BluetoothProfile
import android.bluetooth.BluetoothStatusCodes
import android.content.Context
import android.os.Build
import android.util.Log
import com.convlink.voice.core.FfiLinkDownReason
import com.convlink.voice.core.FfiLinkInfo
import com.convlink.voice.core.FfiLinkKind
import com.convlink.voice.core.FfiLinkRole
import com.convlink.voice.core.FfiLinkUnit
import com.convlink.voice.core.FfiSendOutcome
import com.convlink.voice.engine.LinkRegistry

/**
 * GATT server hosting `MESH_SVC` (DESIGN §9.3): `HELLO` (read, unencrypted, kept current by
 * `SetHello`), `RX` (write without response, central → peripheral) and `TX` (notify,
 * peripheral → central; unencrypted CCCD, because an encrypted one pops the pairing
 * dialog). A central enabling `TX` notifications opens a responder GATT link.
 */
@SuppressLint("MissingPermission") // BLUETOOTH_CONNECT is checked by BleRadio before start
class GattServer(
    private val context: Context,
    private val manager: BluetoothManager,
    private val registry: LinkRegistry,
    private val config: BleConfig,
) {
    private var server: BluetoothGattServer? = null
    private lateinit var tx: BluetoothGattCharacteristic
    private val links = HashMap<String, ServerLink>()
    private val mtus = HashMap<String, Int>()

    @Volatile
    var hello: ByteArray = ByteArray(config.helloLen)

    @Synchronized
    fun start(): Boolean {
        if (server != null) return true
        val s = manager.openGattServer(context, callback) ?: return false
        val svc = BluetoothGattService(config.service, BluetoothGattService.SERVICE_TYPE_PRIMARY)
        svc.addCharacteristic(
            BluetoothGattCharacteristic(config.hello, BluetoothGattCharacteristic.PROPERTY_READ,
                BluetoothGattCharacteristic.PERMISSION_READ),
        )
        svc.addCharacteristic(
            BluetoothGattCharacteristic(config.rx, BluetoothGattCharacteristic.PROPERTY_WRITE_NO_RESPONSE,
                BluetoothGattCharacteristic.PERMISSION_WRITE),
        )
        tx = BluetoothGattCharacteristic(config.tx, BluetoothGattCharacteristic.PROPERTY_NOTIFY, 0).apply {
            addDescriptor(
                BluetoothGattDescriptor(BleConfig.CCCD,
                    BluetoothGattDescriptor.PERMISSION_READ or BluetoothGattDescriptor.PERMISSION_WRITE),
            )
        }
        svc.addCharacteristic(tx)
        if (!s.addService(svc)) {
            s.close()
            return false
        }
        server = s
        return true
    }

    @Synchronized
    fun stop() {
        val ids = links.values.map { it.id }
        links.clear()
        mtus.clear()
        ids.forEach { registry.down(it, FfiLinkDownReason.LocalClose) }
        server?.close()
        server = null
    }

    private fun linkFor(device: BluetoothDevice): ServerLink? = synchronized(this) { links[device.address] }

    private val callback = object : BluetoothGattServerCallback() {
        override fun onConnectionStateChange(device: BluetoothDevice, status: Int, newState: Int) {
            if (newState == BluetoothProfile.STATE_CONNECTED) {
                // The central (e.g. an iPhone) picks 2M by default; ask for 1M for range.
                server?.setPreferredPhy(device, PhyPolicy.PREFERRED_MASK, PhyPolicy.PREFERRED_MASK, PhyPolicy.OPTIONS)
                return
            }
            if (newState != BluetoothProfile.STATE_DISCONNECTED) return
            val link = synchronized(this@GattServer) {
                mtus.remove(device.address)
                links.remove(device.address)
            } ?: return
            registry.down(link.id, LinkDownMapper.disconnect(status, link.localClose))
        }

        override fun onPhyUpdate(device: BluetoothDevice, txPhy: Int, rxPhy: Int, status: Int) {
            Log.i("BlePhy", "server ${device.address} tx=${PhyPolicy.label(txPhy)} rx=${PhyPolicy.label(rxPhy)} status=$status")
        }

        override fun onMtuChanged(device: BluetoothDevice, mtu: Int) {
            synchronized(this@GattServer) { mtus[device.address] = mtu }
            linkFor(device)?.let { registry.metrics(it.id, null, mtu.toUShort()) }
        }

        override fun onCharacteristicReadRequest(
            device: BluetoothDevice, requestId: Int, offset: Int, characteristic: BluetoothGattCharacteristic,
        ) {
            val s = server ?: return
            if (characteristic.uuid != config.hello) {
                s.sendResponse(device, requestId, BluetoothGatt.GATT_READ_NOT_PERMITTED, offset, null)
                return
            }
            val v = hello
            val body = if (offset >= v.size) ByteArray(0) else v.copyOfRange(offset, v.size)
            s.sendResponse(device, requestId, BluetoothGatt.GATT_SUCCESS, offset, body)
        }

        override fun onCharacteristicWriteRequest(
            device: BluetoothDevice, requestId: Int, characteristic: BluetoothGattCharacteristic,
            preparedWrite: Boolean, responseNeeded: Boolean, offset: Int, value: ByteArray?,
        ) {
            val ok = characteristic.uuid == config.rx && !preparedWrite && offset == 0
            if (responseNeeded) {
                server?.sendResponse(device, requestId,
                    if (ok) BluetoothGatt.GATT_SUCCESS else BluetoothGatt.GATT_WRITE_NOT_PERMITTED, offset, null)
            }
            if (ok && value != null) linkFor(device)?.let { registry.data(it.id, value) }
        }

        override fun onDescriptorWriteRequest(
            device: BluetoothDevice, requestId: Int, descriptor: BluetoothGattDescriptor,
            preparedWrite: Boolean, responseNeeded: Boolean, offset: Int, value: ByteArray?,
        ) {
            val isTxCccd = descriptor.uuid == BleConfig.CCCD && descriptor.characteristic.uuid == config.tx
            if (responseNeeded) {
                server?.sendResponse(device, requestId,
                    if (isTxCccd) BluetoothGatt.GATT_SUCCESS else BluetoothGatt.GATT_WRITE_NOT_PERMITTED, offset, null)
            }
            if (!isTxCccd || value == null) return
            val enable = value.contentEquals(BluetoothGattDescriptor.ENABLE_NOTIFICATION_VALUE)
            if (enable) subscribed(device) else linkFor(device)?.let {
                synchronized(this@GattServer) { links.remove(device.address) }
                registry.down(it.id, FfiLinkDownReason.PeerClosed)
            }
        }

        override fun onNotificationSent(device: BluetoothDevice, status: Int) {
            linkFor(device)?.sent()
        }
    }

    private fun subscribed(device: BluetoothDevice) {
        val (link, mtu) = synchronized(this) {
            if (links.containsKey(device.address)) return
            val l = ServerLink(registry.allocateId(), device)
            links[device.address] = l
            l to (mtus[device.address] ?: DEFAULT_MTU)
        }
        registry.up(link, gattInfo(FfiLinkRole.RESPONDER, mtu, config, null))
    }

    /** A central subscribed to `TX`: notifications out, `RX` writes in. */
    private inner class ServerLink(override val id: Int, val device: BluetoothDevice) : MeshLink {
        override val kind = FfiLinkKind.BLE_GATT
        private var inFlight = false
        private var blocked = false

        @Volatile
        var localClose = false

        override fun send(bytes: ByteArray): FfiSendOutcome {
            val s = server ?: return FfiSendOutcome.NO_LINK
            synchronized(this) {
                if (inFlight) {
                    blocked = true
                    return FfiSendOutcome.WOULD_BLOCK
                }
                inFlight = true
            }
            val ok = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
                s.notifyCharacteristicChanged(device, tx, false, bytes) == BluetoothStatusCodes.SUCCESS
            } else {
                @Suppress("DEPRECATION")
                tx.value = bytes
                @Suppress("DEPRECATION")
                s.notifyCharacteristicChanged(device, tx, false)
            }
            if (!ok) {
                synchronized(this) {
                    inFlight = false
                    blocked = true
                }
                return FfiSendOutcome.WOULD_BLOCK
            }
            return FfiSendOutcome.ACCEPTED
        }

        /** `onNotificationSent`: the stack can take the next notification. */
        fun sent() {
            val signal = synchronized(this) {
                inFlight = false
                val b = blocked
                blocked = false
                b
            }
            if (signal) registry.writable(id)
        }

        override fun close(reason: Int) {
            localClose = true
            synchronized(this@GattServer) { links.remove(device.address) }
            server?.cancelConnection(device)
            registry.down(id, FfiLinkDownReason.LocalClose)
        }
    }

    companion object {
        const val DEFAULT_MTU = 23
    }
}

/** LinkInfo of a GATT link: one ATT value per unit, `max_frame` = ATT MTU − 3. */
internal fun gattInfo(role: FfiLinkRole, mtu: Int, config: BleConfig, discId: Int?) = FfiLinkInfo(
    kind = FfiLinkKind.BLE_GATT,
    role = role,
    maxFrame = (mtu - 3).coerceIn(20, config.maxStreamFrame).toUShort(),
    unit = FfiLinkUnit.MESSAGE,
    peerPlatformHint = null,
    attMtu = mtu.toUShort(),
    discId = discId?.toUInt(),
)
