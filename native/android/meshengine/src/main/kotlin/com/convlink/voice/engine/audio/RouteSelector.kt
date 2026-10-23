package com.convlink.voice.engine.audio

import android.media.AudioDeviceInfo
import com.convlink.voice.core.FfiAudioRoute
import com.convlink.voice.core.FfiAudioRouteKind
import com.convlink.voice.core.FfiRoutePreference

/** A communication device as seen by the selector (framework-free, JVM-testable). */
data class RouteDevice(
    val id: Int,
    /** `AudioDeviceInfo.TYPE_*`. */
    val type: Int,
    val sampleRates: IntArray = IntArray(0),
) {
    override fun equals(other: Any?) = other is RouteDevice && id == other.id && type == other.type
    override fun hashCode() = 31 * id + type
}

/**
 * Communication-device choice (DESIGN §10.2.2): `TYPE_BLE_HEADSET` > `TYPE_BLUETOOTH_SCO` >
 * `TYPE_WIRED_HEADSET` / `TYPE_USB_HEADSET` > `TYPE_BUILTIN_SPEAKER` (car profile). The
 * Speaker preference forces the built-in speaker; Headset never picks the speaker while a
 * headset exists.
 */
object RouteSelector {
    private val HEADSET_ORDER = listOf(
        AudioDeviceInfo.TYPE_BLE_HEADSET,
        AudioDeviceInfo.TYPE_BLUETOOTH_SCO,
        AudioDeviceInfo.TYPE_WIRED_HEADSET,
        AudioDeviceInfo.TYPE_USB_HEADSET,
    )

    fun isHeadset(type: Int) = type in HEADSET_ORDER

    fun choose(available: List<RouteDevice>, pref: FfiRoutePreference): RouteDevice? {
        val speaker = available.firstOrNull { it.type == AudioDeviceInfo.TYPE_BUILTIN_SPEAKER }
        if (pref == FfiRoutePreference.SPEAKER) return speaker ?: available.firstOrNull()
        for (t in HEADSET_ORDER) available.firstOrNull { it.type == t }?.let { return it }
        return speaker ?: available.firstOrNull { it.type == AudioDeviceInfo.TYPE_BUILTIN_EARPIECE }
    }

    /** The core's route kind for a device type (HFP width from the SCO device's rates). */
    fun kind(device: RouteDevice?): FfiAudioRouteKind = when (device?.type) {
        null -> FfiAudioRouteKind.NONE
        AudioDeviceInfo.TYPE_BLE_HEADSET -> FfiAudioRouteKind.LE_AUDIO
        AudioDeviceInfo.TYPE_BLUETOOTH_SCO ->
            if (device.sampleRates.isEmpty() || device.sampleRates.any { it >= 16_000 }) {
                FfiAudioRouteKind.HFP_WIDEBAND
            } else {
                FfiAudioRouteKind.HFP_NARROWBAND
            }
        AudioDeviceInfo.TYPE_WIRED_HEADSET, AudioDeviceInfo.TYPE_WIRED_HEADPHONES,
        AudioDeviceInfo.TYPE_USB_HEADSET -> FfiAudioRouteKind.WIRED
        AudioDeviceInfo.TYPE_BUILTIN_SPEAKER -> FfiAudioRouteKind.BUILTIN_SPEAKER
        AudioDeviceInfo.TYPE_BUILTIN_EARPIECE -> FfiAudioRouteKind.BUILTIN_RECEIVER
        else -> FfiAudioRouteKind.NONE
    }

    /** Input rate reported to the core: 16 kHz wideband/LE/wired/built-in, 8 kHz narrowband HFP. */
    fun inputRate(kind: FfiAudioRouteKind, capture: Boolean): UInt = when {
        !capture || kind == FfiAudioRouteKind.NONE -> 0u
        kind == FfiAudioRouteKind.HFP_NARROWBAND -> 8_000u
        else -> 16_000u
    }

    fun route(device: RouteDevice?, available: List<RouteDevice>, capture: Boolean): FfiAudioRoute {
        val kind = kind(device)
        return FfiAudioRoute(
            kind = kind,
            inputRate = inputRate(kind, capture),
            output = kind,
            headsetPresent = available.any { isHeadset(it.type) },
        )
    }
}
