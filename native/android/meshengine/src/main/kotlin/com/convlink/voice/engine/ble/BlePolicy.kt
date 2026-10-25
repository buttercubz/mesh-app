package com.convlink.voice.engine.ble

import android.bluetooth.BluetoothDevice
import android.bluetooth.le.ScanSettings
import com.convlink.voice.core.FfiDiscoveryMode
import com.convlink.voice.core.FfiLinkDownReason

/**
 * LE PHY preference for every BLE link Android takes part in (field test 2026-10-24: the
 * forced 2M PHY cost ~3-5 dB of sensitivity and the link degraded from ~10 m). 1M is
 * requested for TX and RX on both roles (client after connect, server when a central such
 * as an iPhone connects); the PHY update uses the intersection of both sides' preferences,
 * so a 1M-only preference keeps the link off 2M. 1M with DLE carries far more than our
 * 60 kbps per-link pacing. Coded PHY is not used: iPhones cannot scan or connect on it.
 */
object PhyPolicy {
    const val PREFERRED_MASK = BluetoothDevice.PHY_LE_1M_MASK
    const val OPTIONS = BluetoothDevice.PHY_OPTION_NO_PREFERRED

    /** Log label of a `BluetoothDevice.PHY_LE_*` value from onPhyUpdate / onPhyRead. */
    fun label(phy: Int): String = when (phy) {
        BluetoothDevice.PHY_LE_1M -> "1M"
        BluetoothDevice.PHY_LE_2M -> "2M"
        BluetoothDevice.PHY_LE_CODED -> "Coded"
        else -> "?$phy"
    }
}

/**
 * Maps Android GATT / L2CAP failures onto the core's link-down reasons (DESIGN §3.2, §9.5).
 * GATT 133 (`GATT_ERROR`) and 257 (`GATT_FAILURE`) while **connecting** are the controller's
 * connection-limit failures: they become `LimitReached`, which lowers that device's BLE max
 * by 1 for 10 min in the core.
 */
object LinkDownMapper {
    const val GATT_SUCCESS = 0
    const val GATT_CONN_TIMEOUT = 8
    const val GATT_CONN_TERMINATE_PEER_USER = 19
    const val GATT_CONN_TERMINATE_LOCAL_HOST = 22
    const val GATT_CONN_FAIL_ESTABLISH = 62
    const val GATT_ERROR = 133
    const val GATT_FAILURE = 257

    /** BluetoothStatusCodes connection-limit style errors (API 33+ connect callbacks). */
    private val LIMIT_STATUS_CODES = setOf(GATT_ERROR, GATT_FAILURE)

    /** A failure before the link was up (connect, service discovery, HELLO read, subscribe). */
    fun connectFailure(status: Int): FfiLinkDownReason = when (status) {
        in LIMIT_STATUS_CODES -> FfiLinkDownReason.LimitReached
        GATT_CONN_TIMEOUT, GATT_CONN_FAIL_ESTABLISH -> FfiLinkDownReason.Timeout
        GATT_CONN_TERMINATE_PEER_USER -> FfiLinkDownReason.PeerClosed
        GATT_CONN_TERMINATE_LOCAL_HOST -> FfiLinkDownReason.LocalClose
        else -> FfiLinkDownReason.Error(status)
    }

    /** A disconnection of an established link. */
    fun disconnect(status: Int, localClose: Boolean): FfiLinkDownReason = when {
        localClose -> FfiLinkDownReason.LocalClose
        status == GATT_SUCCESS || status == GATT_CONN_TERMINATE_PEER_USER -> FfiLinkDownReason.PeerClosed
        status == GATT_CONN_TIMEOUT -> FfiLinkDownReason.Timeout
        status == GATT_CONN_TERMINATE_LOCAL_HOST -> FfiLinkDownReason.LocalClose
        else -> FfiLinkDownReason.Error(status)
    }
}

/** How hard the radio hunts for peers (scan duty and advertising rate). */
enum class ScanIntensity { HIGH, MEDIUM, LOW }

/**
 * Scan and advertising parameters for one [ScanIntensity] (framework constants, so the
 * pure policy is JVM-testable). [advInterval] is in 0.625 ms units.
 */
data class RadioDuty(
    val intensity: ScanIntensity,
    val scanMode: Int,
    val callbackType: Int,
    val matchMode: Int,
    val advInterval: Int,
)

/**
 * Discovery duty cycle (field test 1, idle saturation): the core keeps `Forming` while any
 * member is unreachable, and a 100 % LOW_LATENCY scan plus 250 ms advertising starves the
 * connection events of the links already up (supervision timeouts → partitions → more
 * Forming). The driver therefore scales the duty with the links it already holds:
 *
 * - `Forming` with **0** BLE links → HIGH: SCAN_MODE_LOW_LATENCY, advertising ≈ 250 ms;
 * - `Forming` with ≥ 1 link (hunting a missing member) → MEDIUM: SCAN_MODE_BALANCED, ≈ 1 s;
 * - `Stable` → LOW: SCAN_MODE_LOW_POWER, ≈ 1 s.
 *
 * Every scan uses MATCH_MODE_STICKY (fewer, steadier reports). CALLBACK_TYPE_FIRST_MATCH is
 * used only at LOW intensity and only when the controller offloads filtering (it reports
 * a device once until lost; every intensity change restarts the scan, which resets it, so
 * hunting modes keep ALL_MATCHES for re-discovery after an Abort). A core-supplied
 * intensity (optional `SetDiscovery.intensity`, see [coreIntensity]) is honoured, except
 * that HIGH is demoted to MEDIUM while a link is up.
 */
object DiscoveryDuty {
    /** AdvertisingSetParameters.INTERVAL_MEDIUM (400 × 0.625 ms = 250 ms). */
    const val ADV_FAST = 400

    /** AdvertisingSetParameters.INTERVAL_HIGH (1600 × 0.625 ms = 1 s). */
    const val ADV_SLOW = 1600

    /** Null when discovery is off. */
    fun intensity(mode: FfiDiscoveryMode, bleLinks: Int, core: ScanIntensity? = null): ScanIntensity? {
        if (mode == FfiDiscoveryMode.OFF) return null
        val own = when {
            mode == FfiDiscoveryMode.STABLE -> ScanIntensity.LOW
            bleLinks <= 0 -> ScanIntensity.HIGH
            else -> ScanIntensity.MEDIUM
        }
        val chosen = core ?: own
        return if (chosen == ScanIntensity.HIGH && bleLinks > 0) ScanIntensity.MEDIUM else chosen
    }

    fun duty(intensity: ScanIntensity, offloadedFiltering: Boolean): RadioDuty = when (intensity) {
        ScanIntensity.HIGH -> RadioDuty(intensity, ScanSettings.SCAN_MODE_LOW_LATENCY,
            ScanSettings.CALLBACK_TYPE_ALL_MATCHES, ScanSettings.MATCH_MODE_STICKY, ADV_FAST)
        ScanIntensity.MEDIUM -> RadioDuty(intensity, ScanSettings.SCAN_MODE_BALANCED,
            ScanSettings.CALLBACK_TYPE_ALL_MATCHES, ScanSettings.MATCH_MODE_STICKY, ADV_SLOW)
        ScanIntensity.LOW -> RadioDuty(intensity, ScanSettings.SCAN_MODE_LOW_POWER,
            if (offloadedFiltering) ScanSettings.CALLBACK_TYPE_FIRST_MATCH else ScanSettings.CALLBACK_TYPE_ALL_MATCHES,
            ScanSettings.MATCH_MODE_STICKY, ADV_SLOW)
    }

    /**
     * Reads the optional scan-intensity field of a `SetDiscovery` command when the
     * generated bindings carry one (core lane: `scan: FfiScanIntensity`, also accepted as
     * `intensity` / `scanIntensity`); absent → null. Matched by enum name so the driver
     * compiles against both binding versions.
     */
    fun coreIntensity(cmd: Any): ScanIntensity? {
        for (name in GETTERS) {
            val getter = runCatching { cmd.javaClass.getMethod(name) }.getOrNull() ?: continue
            val value = runCatching { getter.invoke(cmd) }.getOrNull() ?: return null
            return fromName(value.toString())
        }
        return null
    }

    private val GETTERS = listOf("getScan", "getIntensity", "getScanIntensity")

    fun fromName(name: String): ScanIntensity? = when (name.uppercase().replace("_", "")) {
        "LOWLATENCY", "HIGH", "FAST", "AGGRESSIVE" -> ScanIntensity.HIGH
        "BALANCED", "MEDIUM", "HUNTING" -> ScanIntensity.MEDIUM
        "LOWPOWER", "LOW", "IDLE", "STABLE" -> ScanIntensity.LOW
        else -> null
    }
}

/**
 * Scan quota (DESIGN §9.4): filtered scans restarted every 9 min, and never more than 5
 * starts per 30 s (Android's scan throttling silently stops the 6th). Pure; times in ms.
 */
class ScanPolicy(
    private val quotaStarts: Int = 5,
    private val quotaWindowMs: Long = 30_000,
    private val restartMs: Long = 9 * 60_000,
) {
    private val starts = ArrayDeque<Long>()

    private fun prune(now: Long) {
        while (starts.isNotEmpty() && now - starts.first() >= quotaWindowMs) starts.removeFirst()
    }

    /** True when a start now stays within the quota. */
    fun canStart(now: Long): Boolean {
        prune(now)
        return starts.size < quotaStarts
    }

    /** Milliseconds until the next start is allowed (0 = now). */
    fun delayUntilAllowed(now: Long): Long {
        prune(now)
        if (starts.size < quotaStarts) return 0
        return (starts.first() + quotaWindowMs - now).coerceAtLeast(0)
    }

    /** Records a start; returns false (and records nothing) when it would break the quota. */
    fun recordStart(now: Long): Boolean {
        if (!canStart(now)) return false
        starts.addLast(now)
        return true
    }

    /** A filtered scan started at [startedAt] must be restarted. */
    fun restartDue(startedAt: Long, now: Long): Boolean = now - startedAt >= restartMs

    val restartIntervalMs: Long get() = restartMs
}
