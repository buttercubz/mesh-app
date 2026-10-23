package com.convlink.voice.engine.ble

import com.convlink.voice.core.FfiBleConstants
import java.util.UUID

/**
 * BLE constants used by the Android drivers, taken from the core's `ble_constants()` so the
 * UUIDs and timings have one source of truth (DESIGN §9.3-§9.5, Appendix A).
 */
data class BleConfig(
    val service: UUID,
    val hello: UUID,
    val rx: UUID,
    val tx: UUID,
    val helloLen: Int,
    val maxStreamFrame: Int,
    val preferredAttMtu: Int,
    val connectTimeoutMs: Long,
    val advIntervalFormingMs: Long,
    val advIntervalStableMs: Long,
    val scanRestartMs: Long,
    val scanQuotaStarts: Int,
    val scanQuotaWindowMs: Long,
) {
    companion object {
        /** Client Characteristic Configuration descriptor (Bluetooth SIG). */
        val CCCD: UUID = UUID.fromString("00002902-0000-1000-8000-00805f9b34fb")

        fun from(c: FfiBleConstants) = BleConfig(
            service = UUID.fromString(c.serviceUuid),
            hello = UUID.fromString(c.helloUuid),
            rx = UUID.fromString(c.rxUuid),
            tx = UUID.fromString(c.txUuid),
            helloLen = c.helloLen.toInt(),
            maxStreamFrame = c.maxStreamFrame.toInt(),
            preferredAttMtu = c.preferredAttMtu.toInt(),
            connectTimeoutMs = c.connectTimeoutMs.toLong(),
            advIntervalFormingMs = c.advIntervalFormingMs.toLong(),
            advIntervalStableMs = c.advIntervalStableMs.toLong(),
            scanRestartMs = c.scanRestartMs.toLong(),
            scanQuotaStarts = c.scanQuotaStarts.toInt(),
            scanQuotaWindowMs = c.scanQuotaWindowMs.toLong(),
        )

        /** DESIGN values (used only when the core is unavailable, e.g. lint-time previews). */
        val DESIGN = BleConfig(
            service = UUID.fromString("31B8CD85-9D3A-4622-A465-3CDCA2C32DC5"),
            hello = UUID.fromString("86841842-A7C3-4844-84D2-5B281A3F616C"),
            rx = UUID.fromString("14E91943-7673-440A-979A-F85B4DBD4DBF"),
            tx = UUID.fromString("A76E3168-B4F6-4BE1-8252-35AB8AF17302"),
            helloLen = 16,
            maxStreamFrame = 1024,
            preferredAttMtu = 517,
            connectTimeoutMs = 8_000,
            advIntervalFormingMs = 250,
            advIntervalStableMs = 1_000,
            scanRestartMs = 9 * 60_000,
            scanQuotaStarts = 5,
            scanQuotaWindowMs = 30_000,
        )
    }
}
