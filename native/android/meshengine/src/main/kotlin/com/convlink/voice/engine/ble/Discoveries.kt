package com.convlink.voice.engine.ble

import com.convlink.voice.core.FfiAdvData
import com.convlink.voice.core.FfiDiscovery
import com.convlink.voice.core.FfiPeerPlatform
import com.convlink.voice.core.FfiTransport

/**
 * Pre-connection information from one scan result (DESIGN §9.2, §9.4). Android connects
 * **only** to adverts whose scan response carries our service data with `flags.android = 1`
 * (iPhones initiate towards Android, never the reverse), so anything else is ignored here.
 */
object DiscoveryParser {
    const val FLAG_ANDROID = 0x01
    const val FLAG_LAB = 0x02

    /** The 9-byte service data (`flags · group_tag[4] · node_token[4]`) of an Android peer. */
    fun androidServiceData(serviceData: ByteArray?): ByteArray? {
        if (serviceData == null || serviceData.size < AdvSpec.SERVICE_DATA_LEN) return null
        if (serviceData[0].toInt() and FLAG_ANDROID == 0) return null
        return serviceData.copyOf(AdvSpec.SERVICE_DATA_LEN)
    }

    /** Clamps the framework RSSI (127 = unavailable) to the core's `Option<i16>`. */
    fun rssi(value: Int): Short? = if (value == 127 || value < Short.MIN_VALUE || value > 20) null else value.toShort()

    fun discovery(discId: Int, serviceData: ByteArray, rssi: Int): FfiDiscovery = FfiDiscovery(
        discId = discId.toUInt(),
        transport = FfiTransport.BLE,
        rssi = rssi(rssi),
        adv = FfiAdvData.ServiceData(serviceData),
        platformHint = FfiPeerPlatform.ANDROID,
    )
}

/**
 * Discovery ids per remote device address. Ids are stable while a device keeps its
 * address, so repeated scan results update the same discovery; [reset] runs when a
 * session stops. Thread-safe.
 */
class DiscoveryTable {
    private val byAddress = HashMap<String, Int>()
    private val byId = HashMap<Int, String>()
    private var next = 1

    @Synchronized
    fun idFor(address: String): Int = byAddress.getOrPut(address) {
        var id = next
        while (id == 0 || byId.containsKey(id)) id++
        next = id + 1
        byId[id] = address
        id
    }

    @Synchronized
    fun address(discId: Int): String? = byId[discId]

    @Synchronized
    fun reset() {
        byAddress.clear()
        byId.clear()
        next = 1
    }
}
