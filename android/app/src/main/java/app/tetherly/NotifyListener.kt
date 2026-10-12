package app.tetherly

import android.app.Notification
import android.app.RemoteInput
import android.content.Intent
import android.os.Bundle
import android.service.notification.NotificationListenerService
import android.service.notification.StatusBarNotification
import android.util.Log

/**
 * Phase 1 ingress. Does not read the SMS database. Forwards title/body over
 * the Noise session; OTP extraction happens on the desktop.
 *
 * Also drains quick replies typed on the desktop and fires the notification's
 * own remote-input reply action (in place). The reply text is user content and
 * is never logged.
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
        // A reply may have arrived before this notification was posted.
        drainReplies()
    }

    /**
     * Fire every quick reply the desktop sent us. Matched to the live
     * notification by `uid` (= `sbn.key`); the notification's own remote-input
     * action actually performs the send, so we never synthesise a reply the app
     * did not declare. Unmatched replies are dropped. Manual-required on device
     * (there is no NotificationListener in CI).
     */
    private fun drainReplies() {
        val replies = NativeBridge.takeReplies()
        if (replies.isEmpty()) return
        val active = try {
            activeNotifications
        } catch (_: Exception) {
            return
        }
        for (reply in replies) {
            val sbn = active.firstOrNull { it.key == reply.uid && it.packageName == reply.appId }
            if (sbn == null) {
                Log.i(TAG, "reply for a notification that is no longer posted; dropped")
                continue
            }
            if (!fireRemoteReply(sbn, reply.text)) {
                Log.i(TAG, "notification has no remote-input reply action; dropped")
            }
        }
    }

    private fun fireRemoteReply(sbn: StatusBarNotification, text: String): Boolean {
        val n = sbn.notification ?: return false
        val action = n.actions?.firstOrNull {
            it.remoteInputs?.any { ri -> ri.allowFreeFormInput } == true
        } ?: return false
        val inputs = action.remoteInputs ?: return false
        val intent = Intent()
        val bundle = Bundle()
        for (ri in inputs) bundle.putCharSequence(ri.resultKey, text)
        RemoteInput.addResultsToIntent(inputs, intent, bundle)
        return try {
            action.actionIntent.send(this, 0, intent)
            true
        } catch (_: Exception) {
            false
        }
    }

    companion object {
        private const val TAG = "TetherlyNotify"
    }
}
