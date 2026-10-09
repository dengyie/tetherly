package app.tetherly

import android.content.Context

class Prefs(ctx: Context) {
    private val p = ctx.getSharedPreferences("tetherly", Context.MODE_PRIVATE)

    var desktopAddr: String
        get() = p.getString("desktop_addr", "192.168.1.2:45717") ?: "192.168.1.2:45717"
        set(v) { p.edit().putString("desktop_addr", v).apply() }

    var pin: String
        get() = p.getString("pin", "") ?: ""
        set(v) { p.edit().putString("pin", v).apply() }
}
