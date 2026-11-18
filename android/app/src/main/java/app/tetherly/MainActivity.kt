package app.tetherly

import android.Manifest
import android.content.Intent
import android.os.Build
import android.os.Bundle
import android.provider.Settings
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Button
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import kotlinx.coroutines.delay

/**
 * §171 Android Compose redesign: declarative Material 3 UI replacing the
 * legacy imperative LinearLayout/ScrollView build. Behavior is a one-to-one
 * port of the old view tree — same fields (desktop addr, PIN), same actions
 * (save+connect, notification-listener settings), same 1s connection-status
 * poll — just expressed as state-driven composables.
 */
class MainActivity : ComponentActivity() {
    private lateinit var prefs: Prefs

    private val notifPermission =
        registerForActivityResult(ActivityResultContracts.RequestPermission()) {}

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        prefs = Prefs(this)
        if (Build.VERSION.SDK_INT >= 33) {
            notifPermission.launch(Manifest.permission.POST_NOTIFICATIONS)
        }
        setContent { TetherlyScreen(prefs) }
    }
}

private fun stateLabel(session: NodeSession): String {
    val code = session.state()
    return when (code) {
        0 -> "未连接"
        1 -> "连接中"
        2 -> "已连接"
        3 -> "错误:" + session.lastError()
        else -> "本机库未加载"
    }
}

@Composable
private fun TetherlyScreen(prefs: Prefs) {
    val context = LocalContext.current
    val session = TetherlyApp.instance.session
    var addr by remember { mutableStateOf(prefs.desktopAddr) }
    var pin by remember { mutableStateOf(prefs.pin) }
    // Same 1s poll the old Runnable/postDelayed loop ran; LaunchedEffect
    // cancels it with the composition instead of leaking onto the view tree.
    var statusLine by remember {
        mutableStateOf("device_id: " + session.identity + " · " + stateLabel(session))
    }
    LaunchedEffect(Unit) {
        while (true) {
            delay(1_000)
            statusLine = "device_id: " + session.identity + " · " + stateLabel(session)
        }
    }

    MaterialTheme {
        Surface(modifier = Modifier.fillMaxSize()) {
            Column(
                modifier = Modifier
                    .fillMaxSize()
                    .verticalScroll(rememberScrollState())
                    .padding(16.dp),
                verticalArrangement = Arrangement.spacedBy(12.dp),
            ) {
                Text("Tetherly · Android 通知源", fontSize = 20.sp)
                Text(
                    "不读短信库。各 ROM 请在系统设置里允许通知使用权和自启动。不要用黑保活。",
                    style = MaterialTheme.typography.bodySmall,
                )
                OutlinedTextField(
                    value = addr,
                    onValueChange = { addr = it },
                    label = { Text("桌面 45717 地址") },
                    modifier = Modifier.fillMaxWidth(),
                    singleLine = true,
                )
                OutlinedTextField(
                    value = pin,
                    onValueChange = { pin = it },
                    label = { Text("8 位 PIN（仅首次）") },
                    modifier = Modifier.fillMaxWidth(),
                    singleLine = true,
                )
                Button(onClick = {
                    prefs.desktopAddr = addr
                    prefs.pin = pin
                    session.start()
                }) {
                    Text("保存并连接")
                }
                Button(onClick = {
                    context.startActivity(Intent(Settings.ACTION_NOTIFICATION_LISTENER_SETTINGS))
                }) {
                    Text("打开通知使用权")
                }
                Spacer(Modifier.height(4.dp))
                Text(statusLine, style = MaterialTheme.typography.bodySmall)
            }
        }
    }
}
