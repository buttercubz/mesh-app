package com.convlink.voice.engine.ble

import android.annotation.SuppressLint
import android.bluetooth.BluetoothDevice
import android.bluetooth.BluetoothGatt
import android.bluetooth.BluetoothGattCallback
import android.bluetooth.BluetoothGattCharacteristic
import android.bluetooth.BluetoothGattDescriptor
import android.bluetooth.BluetoothProfile
import android.bluetooth.BluetoothStatusCodes
import android.content.Context
import android.os.Build
import android.util.Log
import android.os.Handler

/** Posting abstraction over the `meshengine-ble` handler (a manual fake in the JVM tests). */
interface Scheduler {
    fun post(r: Runnable)
    fun postDelayed(r: Runnable, ms: Long)
    fun cancel(r: Runnable)
}

class HandlerScheduler(private val handler: Handler) : Scheduler {
    override fun post(r: Runnable) {
        handler.post(r)
    }

    override fun postDelayed(r: Runnable, ms: Long) {
        handler.postDelayed(r, ms)
    }

    override fun cancel(r: Runnable) = handler.removeCallbacks(r)
}

/** GATT client events of one connection, already reduced to what [GattClientLink] needs. */
interface GattEvents {
    fun onConnectionState(status: Int, connected: Boolean)
    fun onMtuChanged(mtu: Int, status: Int)
    fun onServicesDiscovered(status: Int)
    fun onHelloRead(value: ByteArray, status: Int)
    fun onDescriptorWrite(status: Int)
    fun onTxNotification(value: ByteArray)
    fun onRxWritten(status: Int)
    fun onRssi(rssi: Int, status: Int)
}

/** GATT client operations of one connection (DESIGN §9.3), so the link logic is JVM-testable. */
interface GattOps {
    /** Prefer the LE 1M PHY: about 3-5 dB better sensitivity than 2M, i.e. more range (field test 2026-10-24). */
    fun setPreferredPhy1M()
    fun requestMtu(mtu: Int): Boolean
    fun discoverServices(): Boolean

    /** Resolves `MESH_SVC` and its characteristics after discovery; false without `HELLO`. */
    fun resolveService(): Boolean

    /** `RX` (write without response) and `TX` (notify) are both present. */
    val hasDataPath: Boolean
    fun readHello(): Boolean

    /** `setCharacteristicNotification(TX)` + CCCD write; completes in [GattEvents.onDescriptorWrite]. */
    fun enableTxNotifications(): Boolean

    /** One write-without-response to `RX`; completes in [GattEvents.onRxWritten]. */
    fun writeRx(bytes: ByteArray): Boolean
    fun requestConnectionPriority(high: Boolean): Boolean
    fun readRssi(): Boolean

    /** `disconnect()` + `close()`. */
    fun close()
}

/** [GattOps] over a real `BluetoothGatt` (callbacks run on [handler]). */
@SuppressLint("MissingPermission") // BLUETOOTH_CONNECT is checked by BleRadio before connecting
class AndroidGatt private constructor(private val config: BleConfig) : GattOps {
    private var gatt: BluetoothGatt? = null
    private var hello: BluetoothGattCharacteristic? = null
    private var rx: BluetoothGattCharacteristic? = null
    private var tx: BluetoothGattCharacteristic? = null

    override val hasDataPath: Boolean get() = rx != null && tx != null

    override fun setPreferredPhy1M() {
        gatt?.setPreferredPhy(PhyPolicy.PREFERRED_MASK, PhyPolicy.PREFERRED_MASK, PhyPolicy.OPTIONS)
    }

    override fun requestMtu(mtu: Int): Boolean = gatt?.requestMtu(mtu) ?: false

    override fun discoverServices(): Boolean = gatt?.discoverServices() ?: false

    override fun resolveService(): Boolean {
        val svc = gatt?.getService(config.service) ?: return false
        hello = svc.getCharacteristic(config.hello)
        rx = svc.getCharacteristic(config.rx)
        tx = svc.getCharacteristic(config.tx)
        return hello != null
    }

    override fun readHello(): Boolean {
        val g = gatt ?: return false
        return g.readCharacteristic(hello ?: return false)
    }

    override fun enableTxNotifications(): Boolean {
        val g = gatt ?: return false
        val t = tx ?: return false
        if (!g.setCharacteristicNotification(t, true)) return false
        val cccd = t.getDescriptor(BleConfig.CCCD) ?: return false
        val enable = BluetoothGattDescriptor.ENABLE_NOTIFICATION_VALUE
        return if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
            g.writeDescriptor(cccd, enable) == BluetoothStatusCodes.SUCCESS
        } else {
            @Suppress("DEPRECATION")
            cccd.value = enable
            @Suppress("DEPRECATION")
            g.writeDescriptor(cccd)
        }
    }

    override fun writeRx(bytes: ByteArray): Boolean {
        val g = gatt ?: return false
        val c = rx ?: return false
        val type = BluetoothGattCharacteristic.WRITE_TYPE_NO_RESPONSE
        return if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
            g.writeCharacteristic(c, bytes, type) == BluetoothStatusCodes.SUCCESS
        } else {
            @Suppress("DEPRECATION")
            c.writeType = type
            @Suppress("DEPRECATION")
            c.value = bytes
            @Suppress("DEPRECATION")
            g.writeCharacteristic(c)
        }
    }

    override fun requestConnectionPriority(high: Boolean): Boolean = gatt?.requestConnectionPriority(
        if (high) BluetoothGatt.CONNECTION_PRIORITY_HIGH else BluetoothGatt.CONNECTION_PRIORITY_BALANCED,
    ) ?: false

    override fun readRssi(): Boolean = gatt?.readRemoteRssi() ?: false

    override fun close() {
        val g = gatt ?: return
        gatt = null
        g.disconnect()
        g.close()
    }

    private fun callback(events: GattEvents) = object : BluetoothGattCallback() {
        override fun onConnectionStateChange(g: BluetoothGatt, status: Int, newState: Int) =
            events.onConnectionState(status, newState == BluetoothProfile.STATE_CONNECTED)

        override fun onMtuChanged(g: BluetoothGatt, mtu: Int, status: Int) = events.onMtuChanged(mtu, status)

        override fun onServicesDiscovered(g: BluetoothGatt, status: Int) = events.onServicesDiscovered(status)

        override fun onCharacteristicRead(g: BluetoothGatt, c: BluetoothGattCharacteristic, value: ByteArray, status: Int) {
            if (c.uuid == config.hello) events.onHelloRead(value, status)
        }

        @Deprecated("Deprecated in Java")
        override fun onCharacteristicRead(g: BluetoothGatt, c: BluetoothGattCharacteristic, status: Int) {
            if (Build.VERSION.SDK_INT < Build.VERSION_CODES.TIRAMISU && c.uuid == config.hello) {
                @Suppress("DEPRECATION")
                events.onHelloRead(c.value ?: ByteArray(0), status)
            }
        }

        override fun onDescriptorWrite(g: BluetoothGatt, d: BluetoothGattDescriptor, status: Int) =
            events.onDescriptorWrite(status)

        override fun onCharacteristicChanged(g: BluetoothGatt, c: BluetoothGattCharacteristic, value: ByteArray) {
            if (c.uuid == config.tx) events.onTxNotification(value)
        }

        @Deprecated("Deprecated in Java")
        override fun onCharacteristicChanged(g: BluetoothGatt, c: BluetoothGattCharacteristic) {
            if (Build.VERSION.SDK_INT < Build.VERSION_CODES.TIRAMISU && c.uuid == config.tx) {
                @Suppress("DEPRECATION")
                events.onTxNotification(c.value ?: return)
            }
        }

        override fun onCharacteristicWrite(g: BluetoothGatt, c: BluetoothGattCharacteristic, status: Int) =
            events.onRxWritten(status)

        override fun onReadRemoteRssi(g: BluetoothGatt, rssi: Int, status: Int) = events.onRssi(rssi, status)

        // Logged so a field run shows which PHY the link really settled on (logcat tag BlePhy).
        override fun onPhyUpdate(g: BluetoothGatt, txPhy: Int, rxPhy: Int, status: Int) {
            Log.i("BlePhy", "client ${g.device.address} tx=${PhyPolicy.label(txPhy)} rx=${PhyPolicy.label(rxPhy)} status=$status")
        }
    }

    companion object {
        /** `connectGatt` (LE, 1M PHY for range, callbacks on [handler]); null when refused. */
        fun connect(context: Context, device: BluetoothDevice, config: BleConfig, handler: Handler, events: GattEvents): GattOps? {
            val ops = AndroidGatt(config)
            ops.gatt = device.connectGatt(context, false, ops.callback(events), BluetoothDevice.TRANSPORT_LE,
                PhyPolicy.PREFERRED_MASK, handler) ?: return null
            return ops
        }
    }
}
