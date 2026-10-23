package com.convlink.voice.plugin

import android.content.ActivityNotFoundException
import android.content.Intent
import android.net.Uri
import android.os.Build
import android.os.PowerManager
import android.provider.Settings
import android.util.Log
import android.view.WindowManager
import androidx.activity.result.ActivityResult
import androidx.core.app.NotificationManagerCompat
import androidx.core.content.FileProvider
import com.convlink.voice.engine.lab.LabFileStore
import com.convlink.voice.engine.qr.InviteScanner
import com.getcapacitor.JSObject
import com.getcapacitor.PermissionState
import com.getcapacitor.Plugin
import com.getcapacitor.PluginCall
import com.getcapacitor.PluginMethod
import com.getcapacitor.annotation.ActivityCallback
import com.getcapacitor.annotation.PermissionCallback
import java.io.File

/** Permission aliases of C6 (`IntercomPermission`) and their Android mapping. */
internal object PermissionCenter {
    const val MICROPHONE = "microphone"
    const val BLUETOOTH = "bluetooth"
    const val NOTIFICATIONS = "notifications"
    const val CAMERA = "camera"
    const val LOCATION = "location"
    const val NEARBY_WIFI = "nearbyWifi"
    val ALL = listOf(MICROPHONE, BLUETOOTH, NOTIFICATIONS, CAMERA, LOCATION, NEARBY_WIFI)

    /**
     * Aliases that are real runtime permissions on this SDK. The camera never is: invite
     * scanning uses the Play Services code scanner, which needs no CAMERA permission.
     * POST_NOTIFICATIONS and NEARBY_WIFI_DEVICES exist from API 33.
     */
    fun runtime(alias: String, sdk: Int = Build.VERSION.SDK_INT): Boolean = when (alias) {
        CAMERA -> false
        NOTIFICATIONS, NEARBY_WIFI -> sdk >= 33
        MICROPHONE, BLUETOOTH, LOCATION -> true
        else -> false
    }
}

/**
 * The platform-only half of the MeshIntercom bridge (no engine involved): runtime
 * permissions, keep-screen-on, invite QR scanning, Lab export, app settings and the
 * battery-optimisation exemption. [MeshIntercomPlugin] adds the engine methods.
 */
abstract class PlatformPlugin : Plugin() {

    /** Rejects with a mesh-api `ErrorCode` as both message and code (what the UI reads). */
    internal fun reject(call: PluginCall, code: String, detail: String? = null) {
        if (detail != null) Log.w("MeshIntercom", "${call.methodName} rejected: $code ($detail)")
        call.reject(code, code)
    }

    // MARK: permissions

    @PluginMethod
    override fun checkPermissions(call: PluginCall) {
        call.resolve(permissionStatus().also { onPermissionStatus(it, emptyList()) })
    }

    @PluginMethod
    override fun requestPermissions(call: PluginCall) {
        val asked = call.getArray("permissions")?.toList<String>() ?: PermissionCenter.ALL
        if (asked.any { it !in PermissionCenter.ALL }) return reject(call, "invalidParams")
        val aliases = asked.filter { PermissionCenter.runtime(it) && getPermissionState(it) != PermissionState.GRANTED }
        if (aliases.isEmpty()) return call.resolve(permissionStatus().also { onPermissionStatus(it, emptyList()) })
        requestPermissionForAliases(aliases.distinct().toTypedArray(), call, "permissionsAnswered")
    }

    @PermissionCallback
    fun permissionsAnswered(call: PluginCall) {
        val asked = call.getArray("permissions")?.toList<String>() ?: PermissionCenter.ALL
        call.resolve(permissionStatus().also { onPermissionStatus(it, asked) })
    }

    /** Every permission answer the UI gets (diagnostics `perm` notes in the subclass). */
    protected open fun onPermissionStatus(status: JSObject, asked: Collection<String>) {}

    /** Every C6 permission with its state on this SDK. */
    private fun permissionStatus(): JSObject {
        val out = JSObject()
        for (alias in PermissionCenter.ALL) {
            val state = when {
                alias == PermissionCenter.CAMERA -> PermissionState.GRANTED
                alias == PermissionCenter.NOTIFICATIONS && !PermissionCenter.runtime(alias) ->
                    if (NotificationManagerCompat.from(context).areNotificationsEnabled()) PermissionState.GRANTED else PermissionState.DENIED
                // Wi-Fi Aware on API 31-32 is covered by the location permission.
                alias == PermissionCenter.NEARBY_WIFI && !PermissionCenter.runtime(alias) ->
                    getPermissionState(PermissionCenter.LOCATION)
                else -> getPermissionState(alias)
            }
            out.put(alias, state.toString())
        }
        return out
    }

    // MARK: screen

    /** FLAG_KEEP_SCREEN_ON on the activity window; cleared on every pause. */
    @PluginMethod
    fun setKeepScreenOn(call: PluginCall) {
        val on = call.getBoolean("on") ?: return reject(call, "invalidParams")
        activity.runOnUiThread {
            if (on) {
                activity.window.addFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
            } else {
                activity.window.clearFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
            }
            call.resolve()
        }
    }

    // MARK: invite QR

    @PluginMethod
    fun scanInviteQr(call: PluginCall) {
        activity.runOnUiThread {
            InviteScanner(activity).scan { result ->
                when (result) {
                    is InviteScanner.Result.Invite -> call.resolve(JSObject().put("url", result.url).put("cancelled", false))
                    InviteScanner.Result.Cancelled -> call.resolve(JSObject().put("cancelled", true))
                    InviteScanner.Result.NotInvite -> reject(call, "inviteInvalid")
                    is InviteScanner.Result.Failed -> reject(call, "notSupported", result.message)
                }
            }
        }
    }

    // MARK: Lab export

    /**
     * Shares a Lab run file (`.jsonl`/`.csv` of the engine's Lab directory). The file is
     * copied to `cache/lab/`, which the app's FileProvider exposes, and offered through the
     * system share sheet.
     */
    @PluginMethod
    fun shareFile(call: PluginCall) {
        val path = call.getString("path") ?: return reject(call, "invalidParams")
        val mime = call.getString("mime") ?: return reject(call, "invalidParams")
        val source = BridgeSupport.labFile(LabFileStore.create(context).directory, path)
            ?: return reject(call, "invalidParams", "path")
        val shared = try {
            val dir = File(context.cacheDir, "lab").apply { mkdirs() }
            source.copyTo(File(dir, source.name), overwrite = true)
        } catch (e: java.io.IOException) {
            return reject(call, "storageFailed", e.message)
        }
        val uri = FileProvider.getUriForFile(context, "${context.packageName}.fileprovider", shared)
        val send = Intent(Intent.ACTION_SEND)
            .setType(mime)
            .putExtra(Intent.EXTRA_STREAM, uri)
            .addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
        activity.runOnUiThread {
            try {
                activity.startActivity(Intent.createChooser(send, null))
                call.resolve()
            } catch (e: ActivityNotFoundException) {
                reject(call, "notSupported", e.message)
            }
        }
    }

    // MARK: settings and battery

    @PluginMethod
    fun openAppSettings(call: PluginCall) {
        val intent = Intent(Settings.ACTION_APPLICATION_DETAILS_SETTINGS, Uri.fromParts("package", context.packageName, null))
            .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
        activity.runOnUiThread {
            try {
                activity.startActivity(intent)
                call.resolve()
            } catch (e: ActivityNotFoundException) {
                reject(call, "notSupported", e.message)
            }
        }
    }

    /** Asks for the battery-optimisation exemption (DESIGN §10.2); resolves the outcome. */
    @PluginMethod
    fun requestBatteryExemption(call: PluginCall) {
        if (ignoringBatteryOptimizations()) return call.resolve(JSObject().put("granted", true))
        @Suppress("BatteryLife") // Background voice intercom is an accepted use case (§10.2).
        val intent = Intent(Settings.ACTION_REQUEST_IGNORE_BATTERY_OPTIMIZATIONS, Uri.parse("package:${context.packageName}"))
        try {
            startActivityForResult(call, intent, "batteryAnswered")
        } catch (e: ActivityNotFoundException) {
            reject(call, "notSupported", e.message)
        }
    }

    @ActivityCallback
    fun batteryAnswered(call: PluginCall, @Suppress("UNUSED_PARAMETER") result: ActivityResult) {
        call.resolve(JSObject().put("granted", ignoringBatteryOptimizations()))
    }

    private fun ignoringBatteryOptimizations(): Boolean =
        (context.getSystemService(PowerManager::class.java))?.isIgnoringBatteryOptimizations(context.packageName) == true
}
