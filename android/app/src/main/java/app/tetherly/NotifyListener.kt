package app.tetherly

import android.app.Notification
import android.app.RemoteInput
import android.content.Intent
import android.os.Bundle
import android.os.Handler
import android.os.Looper
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
    private val handler = Handler(Looper.getMainLooper())
    private val pendingReplies = QuickReplyQueue()
    private var drainScheduled = false

    override fun onListenerConnected() {
        super.onListenerConnected()
        // ForegroundKeep owns the native session lifecycle, including the
        // Android 15 timeout recovery path. Starting it here avoids a second
        // attach that could replace the service-owned session.
        ForegroundKeep.start(this)
        scheduleDrain()
    }

    override fun onDestroy() {
        handler.removeCallbacksAndMessages(null)
        drainScheduled = false
        pendingReplies.clear()
        TetherlyApp.instance.session.stop()
        ForegroundKeep.stop(this)
        super.onDestroy()
    }

    override fun onListenerDisconnected() {
        // Capture the destructive native inbox before stopping the session.
        // This callback can be transient, so the bounded Kotlin queue survives
        // until the listener reconnects.
        drainReplies()
        handler.removeCallbacksAndMessages(null)
        drainScheduled = false
        TetherlyApp.instance.session.stop()
        ForegroundKeep.stop(this)
        super.onListenerDisconnected()
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
        scheduleDrain(0L)
    }

    override fun onNotificationRemoved(sbn: StatusBarNotification) {
        // The desktop reply may race notification removal. Keep it queued and
        // let the bounded retry window handle a transient disappearance.
        scheduleDrain()
    }

    /**
     * Fire every quick reply the desktop sent us. The native queue is
     * destructive, so take() happens only once and the Kotlin queue owns the
     * retry until the notification action accepts it.
     */
    private fun drainReplies() {
        val now = System.currentTimeMillis()
        // Drain through NodeSession so the listener and foreground service use
        // the same native session ownership boundary.
        pendingReplies.enqueue(TetherlyApp.instance.session.takeReplies(), now)
        if (!pendingReplies.hasPending(now)) {
            // There is no native callback when a reply arrives; keep polling
            // while the listener is connected so a reply does not depend on a
            // later notification-post event.
            scheduleDrain()
            return
        }
        val active = try {
            activeNotifications
        } catch (_: Exception) {
            scheduleDrain()
            return
        }
        for (pending in pendingReplies.takeAttempts(now)) {
            val sbn = active.firstOrNull {
                it.key == pending.reply.uid && it.packageName == pending.reply.appId
            }
            // Count both lookup and action failures. This bounds work even if
            // activeNotifications is stale or the target never exposes input.
            val delivered = sbn != null && fireRemoteReply(sbn, pending.reply.text)
            pendingReplies.finish(pending, delivered, now)
            if (!delivered && pending.attempts >= QuickReplyQueue.DEFAULT_MAX_ATTEMPTS) {
                Log.i(TAG, "quick reply expired before delivery")
            }
        }
        if (pendingReplies.hasPending(now)) scheduleDrain()
    }

    private fun scheduleDrain(delayMs: Long = REPLY_RETRY_INTERVAL_MS) {
        if (drainScheduled) return
        drainScheduled = true
        handler.postDelayed({
            drainScheduled = false
            drainReplies()
        }, delayMs)
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
        private const val REPLY_RETRY_INTERVAL_MS = 2_000L
    }
}
