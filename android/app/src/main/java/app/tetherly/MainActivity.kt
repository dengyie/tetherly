package app.tetherly

import android.Manifest
import android.content.Intent
import android.os.Build
import android.os.Bundle
import android.provider.Settings
import android.widget.Button
import android.widget.EditText
import android.widget.LinearLayout
import android.widget.ScrollView
import android.widget.TextView
import androidx.appcompat.app.AppCompatActivity
import androidx.core.app.ActivityCompat

class MainActivity : AppCompatActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        val prefs = Prefs(this)
        val pad = (16 * resources.displayMetrics.density).toInt()
        val root = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            setPadding(pad, pad, pad, pad)
        }
        fun tv(s: String) = TextView(this).apply { text = s; setPadding(0, pad / 2, 0, pad / 2) }
        root.addView(tv("Tetherly · Android 通知源"))
        root.addView(tv("不读短信库。各 ROM 请在系统设置里允许通知使用权和自启动。不要用黑保活。"))
        val addr = EditText(this).apply {
            setText(prefs.desktopAddr)
            hint = "桌面 45717 地址"
        }
        val pin = EditText(this).apply {
            setText(prefs.pin)
            hint = "8 位 PIN（仅首次）"
            inputType = android.text.InputType.TYPE_CLASS_NUMBER
        }
        root.addView(addr)
        root.addView(pin)
        root.addView(Button(this).apply {
            text = "保存并连接"
            setOnClickListener {
                prefs.desktopAddr = addr.text.toString()
                prefs.pin = pin.text.toString()
                TetherlyApp.instance.session.start()
            }
        })
        root.addView(Button(this).apply {
            text = "打开通知使用权"
            setOnClickListener {
                startActivity(Intent(Settings.ACTION_NOTIFICATION_LISTENER_SETTINGS))
            }
        })
        if (Build.VERSION.SDK_INT >= 33) {
            ActivityCompat.requestPermissions(this, arrayOf(Manifest.permission.POST_NOTIFICATIONS), 1)
        }
        root.addView(tv("device_id: " + TetherlyApp.instance.session.identity.deviceId))
        setContentView(ScrollView(this).apply { addView(root) })
    }
}
