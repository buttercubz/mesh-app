package com.convlink.voice.engine.qr

import android.content.Context
import com.google.mlkit.vision.barcode.common.Barcode
import com.google.mlkit.vision.codescanner.GmsBarcodeScannerOptions
import com.google.mlkit.vision.codescanner.GmsBarcodeScanning

/**
 * Invite QR scanning with the Google code scanner (play-services-code-scanner, DESIGN
 * §10.2.1): the scanner UI runs in Google Play services, so the app needs no CAMERA
 * permission. Only `convlinkvoice://join` URLs are returned; the core parses them.
 */
class InviteScanner(private val context: Context) {
    sealed class Result {
        data class Invite(val url: String) : Result()

        /** A QR code that is not an invite. */
        data object NotInvite : Result()
        data object Cancelled : Result()
        data class Failed(val message: String) : Result()
    }

    fun scan(callback: (Result) -> Unit) {
        val options = GmsBarcodeScannerOptions.Builder()
            .setBarcodeFormats(Barcode.FORMAT_QR_CODE)
            .enableAutoZoom()
            .build()
        GmsBarcodeScanning.getClient(context, options).startScan()
            .addOnSuccessListener { code -> callback(classify(code.rawValue)) }
            .addOnCanceledListener { callback(Result.Cancelled) }
            .addOnFailureListener { e -> callback(Result.Failed(e.message ?: e.javaClass.simpleName)) }
    }

    companion object {
        const val SCHEME_PREFIX = "convlinkvoice://join"

        fun classify(raw: String?): Result {
            val v = raw?.trim() ?: return Result.NotInvite
            return if (v.startsWith(SCHEME_PREFIX, ignoreCase = true)) Result.Invite(v) else Result.NotInvite
        }
    }
}
