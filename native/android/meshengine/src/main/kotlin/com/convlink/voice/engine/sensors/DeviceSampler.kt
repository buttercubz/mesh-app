package com.convlink.voice.engine.sensors

import android.app.KeyguardManager
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.os.BatteryManager
import android.os.PowerManager
import com.convlink.voice.core.FfiThermal

/** One device reading for `FfiPlatformSample`. */
data class DeviceReading(val battery: Double?, val charging: Boolean?, val thermal: FfiThermal?, val locked: Boolean?)

/** Battery, charging, thermal and lock state (DESIGN §3.5 `on_platform`). Any thread. */
class DeviceSampler(private val context: Context) {
    private val power = context.getSystemService(PowerManager::class.java)
    private val keyguard = context.getSystemService(KeyguardManager::class.java)

    fun read(): DeviceReading {
        // Sticky broadcast: no receiver is registered.
        val battery = context.registerReceiver(null, IntentFilter(Intent.ACTION_BATTERY_CHANGED))
        val level = battery?.getIntExtra(BatteryManager.EXTRA_LEVEL, -1) ?: -1
        val scale = battery?.getIntExtra(BatteryManager.EXTRA_SCALE, -1) ?: -1
        val status = battery?.getIntExtra(BatteryManager.EXTRA_STATUS, -1) ?: -1
        return DeviceReading(
            battery = Samples.batteryFraction(level, scale),
            charging = if (status < 0) null else Samples.charging(status),
            thermal = power?.let { Samples.thermal(it.currentThermalStatus) },
            locked = keyguard?.isKeyguardLocked,
        )
    }
}

/** Pure conversions (JVM-tested). */
object Samples {
    fun batteryFraction(level: Int, scale: Int): Double? =
        if (level < 0 || scale <= 0) null else (level.toDouble() / scale).coerceIn(0.0, 1.0)

    fun charging(status: Int): Boolean =
        status == BatteryManager.BATTERY_STATUS_CHARGING || status == BatteryManager.BATTERY_STATUS_FULL

    /** `PowerManager.THERMAL_STATUS_*` → the core's four levels. */
    fun thermal(status: Int): FfiThermal = when {
        status <= PowerManager.THERMAL_STATUS_LIGHT -> FfiThermal.NOMINAL
        status == PowerManager.THERMAL_STATUS_MODERATE -> FfiThermal.FAIR
        status == PowerManager.THERMAL_STATUS_SEVERE -> FfiThermal.SERIOUS
        else -> FfiThermal.CRITICAL
    }
}
