package app.tetherly

import android.app.Application

class TetherlyApp : Application() {
    val session: NodeSession by lazy { NodeSession(this) }

    override fun onCreate() {
        super.onCreate()
        instance = this
        // §172 field finding: a force-stopped or updated app used to sit on
        // the connect form until the user tapped 保存并连接 again. A paired
        // install re-attaches here; the native loop owns retry/backoff, and
        // ForegroundKeep.start from the listener path is idempotent through
        // NodeSession.ensureStarted.
        if (session.shouldAutoStart()) session.ensureStarted()
    }

    companion object {
        lateinit var instance: TetherlyApp
            private set
    }
}
