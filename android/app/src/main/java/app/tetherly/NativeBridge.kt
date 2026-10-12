package app.tetherly

import org.json.JSONArray
import org.json.JSONException

/**
 * Loads `libtetherly_android.so` (crates/tetherly-android, cargo-ndk arm64-v8a)
 * and forwards to it. Everything cryptographic — real Ed25519 identity from
 * persisted secrets, SPAKE2 pairing, Noise IK, `notify.push` framing — happens
 * in Rust. Without the library the app captures notifications but does not open
 * a raw TCP (that would fail the desktop handshake); the UI shows a clear error.
 */
object NativeBridge {
    @Volatile
    var isLoaded: Boolean = false
        private set

    fun ensureLoaded(): Boolean {
        if (isLoaded) return true
        isLoaded = try {
            System.loadLibrary("tetherly_android")
            true
        } catch (_: UnsatisfiedLinkError) {
            false
        }
        return isLoaded
    }

    /** Start/replace the session. Returns "" on success, else an error string. */
    fun attach(dataDir: String, host: String, port: Int, pin: String): String {
        if (!ensureLoaded()) return "libtetherly_android.so 未加载(需 cargo-ndk 构建)"
        return try {
            nativeAttach(dataDir, host, port, pin)
        } catch (t: Throwable) {
            "native attach 失败:${t.message}"
        }
    }

    fun sendNotify(n: PhoneNotify) {
        if (!isLoaded) return
        try {
            nativeSendNotify(n.uid, n.appId, n.appName, n.title, n.body, n.ts)
        } catch (_: Throwable) {
            // dropped; the session loop will surface the state
        }
    }

    fun deviceId(): String = if (isLoaded) nativeDeviceId() else ""
    fun idPk(): ByteArray = if (isLoaded) nativeIdPk() else ByteArray(0)
    fun state(): Int = if (isLoaded) nativeState() else -1
    fun lastError(): String = if (isLoaded) nativeLastError() else ""

    /**
     * Quick replies typed on the desktop since the last call, drained exactly
     * once. The `text` is user content: never log it. Parsing is best-effort —
     * a malformed payload yields an empty list rather than a crash.
     */
    fun takeReplies(): List<PhoneReply> {
        if (!isLoaded) return emptyList()
        val json = try {
            nativeTakeReplies()
        } catch (_: Throwable) {
            return emptyList()
        }
        if (json.isEmpty()) return emptyList()
        return try {
            val arr = JSONArray(json)
            (0 until arr.length()).map { i ->
                val o = arr.getJSONObject(i)
                PhoneReply(
                    uid = o.optString("uid"),
                    appId = o.optString("app_id"),
                    text = o.optString("text"),
                )
            }
        } catch (_: JSONException) {
            emptyList()
        }
    }

    fun stop() {
        if (isLoaded) nativeStop()
    }

    /** true when the peer address string parses; the Rust side validates too. */
    fun splitAddr(addr: String): Pair<String, Int>? {
        val i = addr.lastIndexOf(':')
        if (i <= 0) return null
        val host = addr.substring(0, i)
        val port = addr.substring(i + 1).toIntOrNull() ?: return null
        if (port !in 1..65535) return null
        return host to port
    }

    private external fun nativeAttach(
        dataDir: String,
        host: String,
        port: Int,
        pin: String,
    ): String
    private external fun nativeSendNotify(
        uid: String,
        appId: String,
        appName: String,
        title: String,
        body: String,
        ts: Long,
    )
    private external fun nativeDeviceId(): String
    private external fun nativeIdPk(): ByteArray
    private external fun nativeState(): Int
    private external fun nativeLastError(): String
    private external fun nativeTakeReplies(): String
    private external fun nativeStop()
}

data class PhoneNotify(
    val uid: String,
    val appId: String,
    val appName: String,
    val title: String,
    val body: String,
    val ts: Long,
)

/**
 * A quick reply typed on the desktop, addressed to one notification. `text` is
 * user content and must not be written to logs.
 */
data class PhoneReply(
    val uid: String,
    val appId: String,
    val text: String,
)