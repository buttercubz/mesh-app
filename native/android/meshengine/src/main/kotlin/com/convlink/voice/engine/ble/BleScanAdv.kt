package com.convlink.voice.engine.ble

import android.annotation.SuppressLint
import android.bluetooth.BluetoothAdapter
import android.bluetooth.le.AdvertisingSet
import android.bluetooth.le.AdvertisingSetCallback
import android.bluetooth.le.ScanCallback
import android.bluetooth.le.ScanFilter
import android.bluetooth.le.ScanResult
import android.bluetooth.le.ScanSettings
import android.os.Handler
import android.os.ParcelUuid
import android.os.SystemClock
import android.util.Log

private const val TAG = "MeshBle"

/**
 * Filtered BLE scanning (DESIGN §9.4): `ScanFilter(serviceUuid = MESH_SVC)` always
 * (`neverForLocation`), duty per [DiscoveryDuty], restarted every 9 min, ≤ 5 starts per
 * 30 s ([ScanPolicy]); a duty change waits for the quota instead of being dropped.
 * Every result whose scan response carries Android service data is reported with a
 * stable discovery id; devices the core aborted are ignored for `ignore_s`.
 */
@SuppressLint("MissingPermission") // BLUETOOTH_SCAN is checked by BleRadio before start
class BleScanner(
    private val adapter: BluetoothAdapter,
    private val config: BleConfig,
    private val handler: Handler,
    private val table: DiscoveryTable,
    private val onDiscovery: (discId: Int, serviceData: ByteArray, rssi: Int) -> Unit,
) {
    private val policy = ScanPolicy(config.scanQuotaStarts, config.scanQuotaWindowMs, config.scanRestartMs)
    private var duty: RadioDuty? = null
    private var running = false
    private var startedAt = 0L
    private val ignored = HashMap<String, Long>()

    private val restart = Runnable { restartScan() }

    private val callback = object : ScanCallback() {
        override fun onScanResult(callbackType: Int, result: ScanResult) = handle(result)

        override fun onBatchScanResults(results: MutableList<ScanResult>) = results.forEach { handle(it) }

        override fun onScanFailed(errorCode: Int) {
            Log.w(TAG, "scan failed: $errorCode")
            running = false
            handler.postDelayed(restart, policy.delayUntilAllowed(SystemClock.elapsedRealtime()).coerceAtLeast(1_000))
        }
    }

    /** Applies the scan duty; null stops scanning (handler thread). */
    fun setDuty(d: RadioDuty?) {
        if (d == duty && (running || d == null)) return
        duty = d
        if (d == null) stop() else restartScan()
    }

    /** The duty currently requested (null = off). */
    val currentDuty: RadioDuty? get() = duty

    /** Ignores a device for [seconds] (`Abort{ignore_s}`). */
    fun ignore(address: String, seconds: Int) {
        ignored[address] = SystemClock.elapsedRealtime() + seconds * 1_000L
    }

    fun stop() {
        handler.removeCallbacks(restart)
        if (running) runCatching { adapter.bluetoothLeScanner?.stopScan(callback) }
        running = false
    }

    private fun restartScan() {
        handler.removeCallbacks(restart)
        val d = duty ?: return
        val now = SystemClock.elapsedRealtime()
        val wait = policy.delayUntilAllowed(now)
        if (wait > 0) {
            handler.postDelayed(restart, wait)
            return
        }
        val scanner = adapter.bluetoothLeScanner ?: return
        if (running) runCatching { scanner.stopScan(callback) }
        policy.recordStart(now)
        val filter = ScanFilter.Builder().setServiceUuid(ParcelUuid(config.service)).build()
        val settings = ScanSettings.Builder()
            .setScanMode(d.scanMode)
            .setCallbackType(d.callbackType)
            .setMatchMode(d.matchMode)
            .setNumOfMatches(ScanSettings.MATCH_NUM_ONE_ADVERTISEMENT)
            .setLegacy(true)
            .build()
        try {
            scanner.startScan(listOf(filter), settings, callback)
        } catch (e: IllegalArgumentException) {
            // FIRST_MATCH without offloaded filtering on some stacks: fall back to ALL_MATCHES.
            Log.w(TAG, "scan settings refused ($e); retrying with ALL_MATCHES")
            duty = d.copy(callbackType = ScanSettings.CALLBACK_TYPE_ALL_MATCHES)
            handler.post(restart)
            return
        }
        running = true
        startedAt = now
        handler.postDelayed(restart, policy.restartIntervalMs)
    }

    private fun handle(r: ScanResult) {
        val address = r.device.address
        ignored[address]?.let {
            if (SystemClock.elapsedRealtime() < it) return
            ignored.remove(address)
        }
        val sd = DiscoveryParser.androidServiceData(r.scanRecord?.getServiceData(ParcelUuid(config.service))) ?: return
        onDiscovery(table.idFor(address), sd, r.rssi)
    }
}

/**
 * The legacy connectable scannable advertising set (DESIGN §9.4, [AdvBuilder]), started from
 * the FGS. A new payload or mode restarts the set with the new parameters.
 */
@SuppressLint("MissingPermission") // BLUETOOTH_ADVERTISE is checked by BleRadio before start
class BleAdvertiser(private val adapter: BluetoothAdapter, private val config: BleConfig) {
    private var current: AdvSpec? = null
    private var set: AdvertisingSet? = null
    private var starting = false

    private val callback = object : AdvertisingSetCallback() {
        override fun onAdvertisingSetStarted(s: AdvertisingSet?, txPower: Int, status: Int) {
            starting = false
            if (status == ADVERTISE_SUCCESS) set = s else Log.w(TAG, "advertising failed: $status")
        }

        override fun onAdvertisingSetStopped(s: AdvertisingSet?) {
            if (s == set) set = null
        }
    }

    /** Starts, updates or (with null) stops advertising (handler thread). */
    fun apply(spec: AdvSpec?) {
        if (spec == current && (set != null || starting)) return
        stop()
        current = spec
        spec ?: return
        val adv = adapter.bluetoothLeAdvertiser ?: return
        starting = true
        adv.startAdvertisingSet(
            AdvBuilder.toParameters(spec), AdvBuilder.toAdvertiseData(spec), AdvBuilder.toScanResponse(spec),
            null, null, callback,
        )
    }

    fun stop() {
        current = null
        starting = false
        runCatching { adapter.bluetoothLeAdvertiser?.stopAdvertisingSet(callback) }
        set = null
    }

    val spec: AdvSpec? get() = current
}
