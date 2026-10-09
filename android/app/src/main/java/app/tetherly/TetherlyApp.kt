package app.tetherly

import android.app.Application

class TetherlyApp : Application() {
    val session: NodeSession by lazy { NodeSession(this) }

    override fun onCreate() {
        super.onCreate()
        instance = this
    }

    companion object {
        lateinit var instance: TetherlyApp
            private set
    }
}
