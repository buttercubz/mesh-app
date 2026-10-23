package com.convlink.voice.engine.ble

import android.bluetooth.le.AdvertiseData
import android.bluetooth.le.AdvertisingSetParameters
import android.os.ParcelUuid
import com.convlink.voice.core.FfiDiscoveryMode

/**
 * Android advertising payloads (DESIGN §9.4): a **legacy, connectable, scannable `ADV_IND`**
 * with Flags + the 128-bit `MESH_SVC` UUID (21 B), and a scan response carrying Service
 * Data 128-bit (AD 0x21) `MESH_SVC` + the core's 9 bytes `flags · group_tag[4] ·
 * node_token[4]` (27 B). No mesh data is ever advertised.
 *
 * [AdvSpec] is the pure description (unit-tested); [toParameters] / [toAdvertiseData] /
 * [toScanResponse] turn it into framework objects.
 */
data class AdvSpec(
    val legacy: Boolean,
    val connectable: Boolean,
    val scannable: Boolean,
    /** In 0.625 ms units (AdvertisingSetParameters.INTERVAL_*). */
    val interval: Int,
    val txPower: Int,
    val includeDeviceName: Boolean,
    val includeTxPower: Boolean,
    val serviceUuid: java.util.UUID,
    val serviceData: ByteArray,
) {
    /** Primary ADV bytes: Flags (3) + Complete List of 128-bit UUIDs (2 + 16). */
    val primaryLength: Int get() = FLAGS_AD_LEN + 2 + UUID_LEN

    /** Scan response bytes: AD header (2) + UUID (16) + service data. */
    val scanResponseLength: Int get() = 2 + UUID_LEN + serviceData.size

    override fun equals(other: Any?): Boolean = other is AdvSpec && legacy == other.legacy &&
        connectable == other.connectable && scannable == other.scannable && interval == other.interval &&
        txPower == other.txPower && includeDeviceName == other.includeDeviceName &&
        includeTxPower == other.includeTxPower && serviceUuid == other.serviceUuid &&
        serviceData.contentEquals(other.serviceData)

    override fun hashCode(): Int = 31 * interval + serviceData.contentHashCode()

    companion object {
        const val UUID_LEN = 16
        const val FLAGS_AD_LEN = 3

        /** Legacy advertising PDU limit. */
        const val LEGACY_MAX = 31

        /** Service data length the core produces for Android (§9.4). */
        const val SERVICE_DATA_LEN = 9
    }
}

object AdvBuilder {
    /**
     * ≈ 250 ms only while Forming with no link up (HIGH duty), ≈ 1 s otherwise
     * ([DiscoveryDuty]): fast advertising next to live links steals their connection events.
     */
    fun interval(mode: FfiDiscoveryMode, bleLinks: Int = 0): Int {
        val i = DiscoveryDuty.intensity(mode, bleLinks) ?: return AdvertisingSetParameters.INTERVAL_HIGH
        return DiscoveryDuty.duty(i, offloadedFiltering = false).advInterval
    }

    fun spec(config: BleConfig, mode: FfiDiscoveryMode, serviceData: ByteArray): AdvSpec =
        spec(config, interval(mode), serviceData)

    /** [interval] in 0.625 ms units (from [RadioDuty.advInterval]). */
    fun spec(config: BleConfig, interval: Int, serviceData: ByteArray): AdvSpec {
        require(serviceData.size == AdvSpec.SERVICE_DATA_LEN) {
            "scan response service data must be ${AdvSpec.SERVICE_DATA_LEN} bytes, got ${serviceData.size}"
        }
        val spec = AdvSpec(
            legacy = true,
            connectable = true,
            scannable = true,
            interval = interval,
            txPower = AdvertisingSetParameters.TX_POWER_HIGH,
            includeDeviceName = false,
            includeTxPower = false,
            serviceUuid = config.service,
            serviceData = serviceData.copyOf(),
        )
        check(spec.primaryLength <= AdvSpec.LEGACY_MAX && spec.scanResponseLength <= AdvSpec.LEGACY_MAX)
        return spec
    }

    fun toParameters(s: AdvSpec): AdvertisingSetParameters = AdvertisingSetParameters.Builder()
        .setLegacyMode(s.legacy)
        .setConnectable(s.connectable)
        .setScannable(s.scannable)
        .setInterval(s.interval)
        .setTxPowerLevel(s.txPower)
        .setIncludeTxPower(s.includeTxPower)
        .build()

    fun toAdvertiseData(s: AdvSpec): AdvertiseData = AdvertiseData.Builder()
        .setIncludeDeviceName(s.includeDeviceName)
        .setIncludeTxPowerLevel(false)
        .addServiceUuid(ParcelUuid(s.serviceUuid))
        .build()

    fun toScanResponse(s: AdvSpec): AdvertiseData = AdvertiseData.Builder()
        .setIncludeDeviceName(false)
        .setIncludeTxPowerLevel(false)
        .addServiceData(ParcelUuid(s.serviceUuid), s.serviceData)
        .build()
}
