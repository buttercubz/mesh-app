package com.convlink.voice.engine.notify

import android.Manifest
import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.os.Build
import androidx.core.app.NotificationCompat
import androidx.core.app.NotificationManagerCompat
import androidx.core.content.ContextCompat
import com.convlink.voice.core.FfiNotification
import com.convlink.voice.core.FfiNotificationCategory
import com.convlink.voice.engine.MeshEngineService
import org.json.JSONObject
import java.util.Locale

/**
 * The ongoing FGS notification (Mute / Talk latch / Leave / Resume) and the engine's
 * notices (`PlatformHooks.notify`, e.g. `engineCrashed`, "Tap to rejoin") — DESIGN
 * §10.2.1, §10.2.3. Actions are delivered to [MeshEngineService]; Resume uses a
 * foreground-service PendingIntent (the notification-interaction exemption lets it start
 * the microphone FGS from the background).
 */
class IntercomNotifications(private val context: Context, private val smallIcon: Int) {
    private val nm = context.getSystemService(NotificationManager::class.java)

    @Volatile
    var catalog: NotificationCatalog = NotificationCatalog.fallback(Locale.getDefault())

    /** Opens the app (set by the bridge, usually the launcher activity). */
    @Volatile
    var contentIntent: PendingIntent? = null

    fun ensureChannels() {
        val fb = catalog.title
        nm.createNotificationChannel(
            NotificationChannel(CHANNEL_ONGOING, fb, NotificationManager.IMPORTANCE_LOW).apply {
                setShowBadge(false)
                setSound(null, null)
            },
        )
        nm.createNotificationChannel(
            NotificationChannel(CHANNEL_ALERTS, fb, NotificationManager.IMPORTANCE_HIGH),
        )
    }

    /** The ongoing notification for `startForeground`. */
    fun ongoing(state: IntercomState): Notification {
        ensureChannels()
        val m = NotificationModels.ongoing(state, catalog)
        val b = NotificationCompat.Builder(context, CHANNEL_ONGOING)
            .setSmallIcon(smallIcon)
            .setContentTitle(m.title)
            .setContentText(m.text)
            .setOngoing(true)
            .setOnlyAlertOnce(true)
            .setCategory(NotificationCompat.CATEGORY_SERVICE)
            .setForegroundServiceBehavior(NotificationCompat.FOREGROUND_SERVICE_IMMEDIATE)
            .setVisibility(NotificationCompat.VISIBILITY_PUBLIC)
        contentIntent?.let { b.setContentIntent(it) }
        m.actions.forEachIndexed { i, (action, label) -> b.addAction(0, label, actionIntent(action, i)) }
        return b.build()
    }

    /** Re-posts the ongoing notification (state change while the FGS runs). */
    fun update(state: IntercomState) {
        if (canPost()) nm.notify(ID_ONGOING, ongoing(state))
    }

    /** `PlatformHooks.notify`: an i18n key + params; unknown keys are skipped. */
    fun post(n: FfiNotification): Boolean {
        val prefix = if (n.category == FfiNotificationCategory.ERROR) "errors." else "notices."
        val params = runCatching { JSONObject(n.paramsJson) }.getOrNull()
        val body = catalog.text(prefix + n.key, params) ?: catalog.text(n.key, params) ?: return false
        return alert(ID_NOTICE_BASE + (n.key.hashCode() and 0xFFF), body, null)
    }

    /** "Mic paused: tap to resume" (ladder L2) with a Resume action. */
    fun micPaused(): Boolean = alert(
        ID_MIC_PAUSED,
        catalog.text("notify.micPaused") ?: NotificationCatalog.fallback(Locale.getDefault()).text("notify.micPaused")!!,
        IntercomAction.RESUME,
    )

    /** "Tap to rejoin" after process death (START_NOT_STICKY, the next launch offers Resume ride). */
    fun rejoin(): Boolean = alert(
        ID_REJOIN,
        catalog.text("notify.rejoin") ?: NotificationCatalog.fallback(Locale.getDefault()).text("notify.rejoin")!!,
        null,
    )

    fun cancelAlerts() {
        nm.cancel(ID_MIC_PAUSED)
    }

    fun cancelAll() {
        nm.cancel(ID_ONGOING)
        nm.cancel(ID_MIC_PAUSED)
    }

    private fun alert(id: Int, body: String, action: IntercomAction?): Boolean {
        if (!canPost()) return false
        ensureChannels()
        val b = NotificationCompat.Builder(context, CHANNEL_ALERTS)
            .setSmallIcon(smallIcon)
            .setContentTitle(catalog.title)
            .setContentText(body)
            .setAutoCancel(true)
            .setPriority(NotificationCompat.PRIORITY_HIGH)
            .setCategory(NotificationCompat.CATEGORY_STATUS)
        contentIntent?.let { b.setContentIntent(it) }
        action?.let { b.addAction(0, catalog.label(it), actionIntent(it, 10)) }
        nm.notify(id, b.build())
        return true
    }

    private fun canPost(): Boolean = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
        ContextCompat.checkSelfPermission(context, Manifest.permission.POST_NOTIFICATIONS) ==
            PackageManager.PERMISSION_GRANTED
    } else {
        NotificationManagerCompat.from(context).areNotificationsEnabled()
    }

    private fun actionIntent(action: IntercomAction, requestCode: Int): PendingIntent {
        val intent = Intent(context, MeshEngineService::class.java).setAction(action.intentAction)
        val flags = PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE
        return if (action == IntercomAction.RESUME) {
            PendingIntent.getForegroundService(context, requestCode, intent, flags)
        } else {
            PendingIntent.getService(context, requestCode, intent, flags)
        }
    }

    companion object {
        const val CHANNEL_ONGOING = "meshengine.intercom"
        const val CHANNEL_ALERTS = "meshengine.alerts"
        const val ID_ONGOING = 0x4D45
        const val ID_MIC_PAUSED = 0x4D46
        const val ID_REJOIN = 0x4D47
        const val ID_NOTICE_BASE = 0x5000
    }
}
