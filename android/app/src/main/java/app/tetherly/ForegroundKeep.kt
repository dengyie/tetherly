package app.tetherly

import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import android.os.Build
import android.os.Handler
import android.os.IBinder
import android.os.Looper
import androidx.core.app.NotificationCompat
import java.util.concurrent.atomic.AtomicBoolean

enum class ForegroundKeepState {
    STOPPED,
    STARTING,
    RUNNING,
    TIMED_OUT,
    RESTART_PENDING,
    FAILED,
}

/** Small, synchronized lifecycle model that can be checked without Android. */
internal class ForegroundKeepStateMachine {
    @Volatile
    var state: ForegroundKeepState = ForegroundKeepState.STOPPED
        private set

    @Synchronized
    fun markStarting(): Boolean {
        if (state == ForegroundKeepState.STARTING || state == ForegroundKeepState.RUNNING) {
            return false
        }
        state = ForegroundKeepState.STARTING
        return true
    }

    @Synchronized
    fun markCreated(): Boolean {
        if (state == ForegroundKeepState.STOPPED) state = ForegroundKeepState.STARTING
        return state == ForegroundKeepState.STARTING
    }

    @Synchronized
    fun markRunning(): Boolean {
        if (state != ForegroundKeepState.STARTING) return false
        state = ForegroundKeepState.RUNNING
        return true
    }

    @Synchronized
    fun markTimedOut(): Boolean {
        if (state != ForegroundKeepState.RUNNING) return false
        state = ForegroundKeepState.TIMED_OUT
        return true
    }

    @Synchronized
    fun markRestartPending() {
        state = ForegroundKeepState.RESTART_PENDING
    }

    @Synchronized
    fun markFailed() {
        state = ForegroundKeepState.FAILED
    }

    @Synchronized
    fun markStopped() {
        state = ForegroundKeepState.STOPPED
    }
}

/** Documented foreground service. No black keep-alive. */
class ForegroundKeep : Service() {
    override fun onBind(intent: Intent?): IBinder? = null

    override fun onCreate() {
        super.onCreate()
        if (stopRequested.get() || !lifecycle.markCreated()) {
            stopSelf()
            return
        }
        try {
            val nm = getSystemService(NotificationManager::class.java)
            if (Build.VERSION.SDK_INT >= 26) {
                nm.createNotificationChannel(
                    NotificationChannel(CH, "Tetherly", NotificationManager.IMPORTANCE_LOW),
                )
            }
            val open = PendingIntent.getActivity(
                this, 0, Intent(this, MainActivity::class.java),
                PendingIntent.FLAG_IMMUTABLE,
            )
            val n = NotificationCompat.Builder(this, CH)
                .setSmallIcon(android.R.drawable.stat_notify_sync)
                .setContentTitle("Tetherly")
                .setContentText("通知转发已开启")
                .setContentIntent(open)
                .setOngoing(true)
                .build()
            if (Build.VERSION.SDK_INT >= 29) {
                startForeground(NOTIFICATION_ID, n, ServiceInfo.FOREGROUND_SERVICE_TYPE_DATA_SYNC)
            } else {
                startForeground(NOTIFICATION_ID, n)
            }
        } catch (_: Throwable) {
            lifecycle.markFailed()
            stopSelf()
        }
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        // Android 15 may call the two-argument onTimeout() for dataSync.
        // START_STICKY also covers process death; the state gate prevents a
        // stale command from attaching a second native session after stop().
        if (!lifecycle.markRunning()) return START_NOT_STICKY
        return try {
            // Idempotent: Application.onCreate auto-starts a paired install on
            // process recreation, and ensureStarted skips the redundant
            // attach instead of tearing down and redialing.
            TetherlyApp.instance.session.ensureStarted()
            START_STICKY
        } catch (_: Throwable) {
            lifecycle.markFailed()
            START_NOT_STICKY
        }
    }

    /** Kept for the shortService callback shape on API 35. */
    override fun onTimeout(startId: Int) {
        handleTimeout(startId)
    }

    /** dataSync/mediaProcessing timeout callback introduced for target 35. */
    override fun onTimeout(startId: Int, fgsType: Int) {
        handleTimeout(startId)
    }

    private fun handleTimeout(startId: Int) {
        if (!lifecycle.markTimedOut()) return
        TetherlyApp.instance.session.stop()
        stopForeground(STOP_FOREGROUND_REMOVE)
        RestartCoordinator.schedule(applicationContext)
        lifecycle.markRestartPending()
        stopSelf(startId)
    }

    override fun onDestroy() {
        TetherlyApp.instance.session.stop()
        if (lifecycle.state != ForegroundKeepState.RESTART_PENDING) lifecycle.markStopped()
        super.onDestroy()
    }

    companion object {
        private const val CH = "tetherly"
        private const val NOTIFICATION_ID = 1
        private const val RESTART_DELAY_MS = 1_000L
        private val lifecycle = ForegroundKeepStateMachine()
        private val stopRequested = AtomicBoolean(false)

        /** Current service lifecycle, safe to read from the Compose poller. */
        val state: ForegroundKeepState
            get() = lifecycle.state

        fun start(ctx: Context) {
            stopRequested.set(false)
            if (!lifecycle.markStarting()) return
            RestartCoordinator.cancel()
            try {
                // Attach before the asynchronous service callback so a
                // notification arriving immediately after listener connect is
                // not silently dropped. Idempotent: Application.onCreate may
                // already have auto-started a paired install.
                TetherlyApp.instance.session.ensureStarted()
                val i = Intent(ctx, ForegroundKeep::class.java)
                if (Build.VERSION.SDK_INT >= 26) ctx.startForegroundService(i) else ctx.startService(i)
            } catch (_: Throwable) {
                TetherlyApp.instance.session.stop()
                lifecycle.markFailed()
            }
        }

        fun stop(ctx: Context) {
            RestartCoordinator.cancel()
            stopRequested.set(true)
            lifecycle.markStopped()
            ctx.stopService(Intent(ctx, ForegroundKeep::class.java))
        }
    }

    private object RestartCoordinator {
        private val handler = Handler(Looper.getMainLooper())
        private var token = 0L
        private var pending: Runnable? = null

        @Synchronized
        fun schedule(ctx: Context) {
            pending?.let(handler::removeCallbacks)
            val current = ++token
            val app = ctx.applicationContext
            val restart = Runnable {
                synchronized(this) {
                    if (current != token) return@Runnable
                    pending = null
                }
                ForegroundKeep.start(app)
            }
            pending = restart
            handler.postDelayed(restart, RESTART_DELAY_MS)
        }

        @Synchronized
        fun cancel() {
            ++token
            pending?.let(handler::removeCallbacks)
            pending = null
        }
    }
}
