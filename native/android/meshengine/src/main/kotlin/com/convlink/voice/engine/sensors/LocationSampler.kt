package com.convlink.voice.engine.sensors

import android.Manifest
import android.annotation.SuppressLint
import android.content.Context
import android.content.pm.PackageManager
import android.location.Location
import android.location.LocationListener
import android.location.LocationManager
import android.os.Looper
import android.os.SystemClock
import androidx.core.content.ContextCompat
import com.convlink.voice.core.FfiGpsFix

/**
 * GPS fixes for distance sharing and Lab GPS runs (DESIGN §10.2.1). Runs only while the
 * session shares location **and** fine location is granted (the FGS then carries the
 * location type, §10.2.3). The latest fresh fix goes into each platform sample.
 */
class LocationSampler(private val context: Context) {
    private val lm = context.getSystemService(LocationManager::class.java)

    @Volatile
    private var last: Location? = null
    private var active = false

    private val listener = LocationListener { loc -> last = loc }

    fun granted(): Boolean =
        ContextCompat.checkSelfPermission(context, Manifest.permission.ACCESS_FINE_LOCATION) ==
            PackageManager.PERMISSION_GRANTED

    val isActive: Boolean get() = active

    /** Starts 1 Hz GPS updates; false when not granted or no provider. Main thread. */
    @SuppressLint("MissingPermission") // checked by granted()
    fun start(): Boolean {
        if (active) return true
        if (!granted() || lm == null || !lm.isProviderEnabled(LocationManager.GPS_PROVIDER)) return false
        lm.requestLocationUpdates(LocationManager.GPS_PROVIDER, INTERVAL_MS, 0f, listener, Looper.getMainLooper())
        active = true
        return true
    }

    fun stop() {
        if (!active) return
        lm?.removeUpdates(listener)
        active = false
        last = null
    }

    /** The latest fix if younger than [maxAgeMs]. */
    fun fix(maxAgeMs: Long = MAX_AGE_MS): FfiGpsFix? {
        val l = last ?: return null
        val ageMs = (SystemClock.elapsedRealtimeNanos() - l.elapsedRealtimeNanos) / 1_000_000
        if (ageMs > maxAgeMs) return null
        return toFix(l.latitude, l.longitude, if (l.hasSpeed()) l.speed else null,
            if (l.hasAccuracy()) l.accuracy else null, if (l.hasBearing()) l.bearing else null)
    }

    companion object {
        const val INTERVAL_MS = 1_000L
        const val MAX_AGE_MS = 5_000L

        fun toFix(lat: Double, lon: Double, speed: Float?, acc: Float?, bearing: Float?) = FfiGpsFix(
            lat = lat,
            lon = lon,
            speedMps = speed?.toDouble(),
            accM = acc?.toDouble(),
            headingDeg = bearing?.toDouble(),
        )
    }
}
