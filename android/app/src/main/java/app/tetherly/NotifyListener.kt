package app.tetherly

import android.service.notification.NotificationListenerService
import android.service.notification.StatusBarNotification
import android.app.Notification

/**
 * Phase 1 ingress. Does not read the SMS database. Forwards title/body over
 * the Noise session; OTP extraction happens on the desktop.
 */
class NotifyListener : NotificationListenerService() {
    override fun onListenerConnected() {
        super.onListenerConnected()
        TetherlyApp.instance.session.start()
        ForegroundKeep.start(this)
    }

    override fun onNotificationPosted(sbn: StatusBarNotification) {
        val n = sbn.notification ?: return
        val extras = n.extras
        val title = extras.getCharSequence(Notification.EXTRA_TITLE)?.toString().orEmpty()
        val text = extras.getCharSequence(Notification.EXTRA_TEXT)?.toString().orEmpty()
        val big = extras.getCharSequence(Notification.EXTRA_BIG_TEXT)?.toString().orEmpty()
        val body = if (big.isNotEmpty()) big else text
        val appName = try {
            packageManager.getApplicationLabel(packageManager.getApplicationInfo(sbn.packageName, 0)).toString()
        } catch (_: Exception) {
            sbn.packageName
        }
        TetherlyApp.instance.session.enqueueNotify(
            PhoneNotify(
                uid = sbn.key,
                appId = sbn.packageName,
                appName = appName,
                title = title.take(4096),
                body = body.take(4096),
                ts = sbn.postTime,
            ),
        )
    }
}
