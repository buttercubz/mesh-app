package com.convlink.voice.engine.radio

import android.Manifest
import android.bluetooth.BluetoothAdapter
import android.bluetooth.BluetoothManager
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.content.pm.PackageManager
import android.net.wifi.aware.WifiAwareManager
import android.provider.Settings
import androidx.core.content.ContextCompat
import com.convlink.voice.core.FfiRadioAvail

/** Pure mapping of the framework state onto the core's `RadioAvail` (DESIGN §3.2). */
object RadioStates {
    /**
     * Bluetooth: `Unsupported` without an adapter, `Unauthorized` without the Nearby devices
     * permissions, `Off` while the adapter is not ON (turning off counts as off) or in
     * airplane mode with Bluetooth off, else `On`.
     */
    fun ble(hasAdapter: Boolean, adapterState: Int, permitted: Boolean): FfiRadioAvail = when {
        !hasAdapter -> FfiRadioAvail.UNSUPPORTED
        !permitted -> FfiRadioAvail.UNAUTHORIZED
        adapterState == BluetoothAdapter.STATE_ON -> FfiRadioAvail.ON
        else -> FfiRadioAvail.OFF
    }

    /** Wi-Fi Aware (Phase 4): `Off` in airplane mode or when the service is unavailable. */
    fun aware(supported: Boolean, available: Boolean, airplane: Boolean): FfiRadioAvail = when {
        !supported -> FfiRadioAvail.UNSUPPORTED
        airplane || !available -> FfiRadioAvail.OFF
        else -> FfiRadioAvail.ON
    }
}

/**
 * `BluetoothAdapter.ACTION_STATE_CHANGED` + airplane mode (+ Wi-Fi Aware state) →
 * `on_platform` RadioState (DESIGN §10.2.1). The listener is called on the main thread
 * whenever either availability changes; the core marks BLE links down with `RadioOff`,
 * holds discovery and raises `bluetoothOff` / `bluetoothDenied`.
 */
class RadioStateMonitor(private val context: Context, private val listener: (FfiRadioAvail, FfiRadioAvail) -> Unit) {
    private val adapter: BluetoothAdapter? = context.getSystemService(BluetoothManager::class.java)?.adapter
    private val awareSupported = context.packageManager.hasSystemFeature(PackageManager.FEATURE_WIFI_AWARE)
    private var registered = false

    @Volatile
    var ble: FfiRadioAvail = FfiRadioAvail.OFF
        private set

    @Volatile
    var aware: FfiRadioAvail = FfiRadioAvail.UNSUPPORTED
        private set

    private val receiver = object : BroadcastReceiver() {
        override fun onReceive(c: Context, intent: Intent) = refresh()
    }

    fun start() {
        if (!registered) {
            val filter = IntentFilter().apply {
                addAction(BluetoothAdapter.ACTION_STATE_CHANGED)
                addAction(Intent.ACTION_AIRPLANE_MODE_CHANGED)
                addAction(WifiAwareManager.ACTION_WIFI_AWARE_STATE_CHANGED)
            }
            ContextCompat.registerReceiver(context, receiver, filter, ContextCompat.RECEIVER_NOT_EXPORTED)
            registered = true
        }
        refresh()
    }

    fun stop() {
        if (registered) {
            context.unregisterReceiver(receiver)
            registered = false
        }
    }

    fun permitted(): Boolean = listOf(
        Manifest.permission.BLUETOOTH_SCAN,
        Manifest.permission.BLUETOOTH_ADVERTISE,
        Manifest.permission.BLUETOOTH_CONNECT,
    ).all { ContextCompat.checkSelfPermission(context, it) == PackageManager.PERMISSION_GRANTED }

    /** Re-reads the state and notifies on change. */
    fun refresh() {
        val airplane = Settings.Global.getInt(context.contentResolver, Settings.Global.AIRPLANE_MODE_ON, 0) != 0
        val b = RadioStates.ble(adapter != null, adapter?.state ?: BluetoothAdapter.STATE_OFF, permitted())
        val awareMgr = if (awareSupported) context.getSystemService(WifiAwareManager::class.java) else null
        val a = RadioStates.aware(awareMgr != null, awareMgr?.isAvailable == true, airplane)
        if (b != ble || a != aware) {
            ble = b
            aware = a
            listener(b, a)
        }
    }
}
