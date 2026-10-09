package app.tetherly

import android.util.Base64
import org.json.JSONArray
import org.json.JSONObject
import java.io.DataInputStream
import java.io.DataOutputStream
import java.io.InputStream
import java.io.OutputStream
import java.security.MessageDigest
import java.security.SecureRandom

/** Length-prefixed bytes. Noise/SPAKE2 live in the desktop node; Android
 *  Phase 1 uses the same Hello then a pairing/resume handshake driven by
 *  the rust node when the phone dials. Until uniffi lands, this client
 *  speaks Hello + Noise via JNI-free rustc cargo-ndk binary later.
 *  For Phase 1 CI we keep a JSON control helper used by unit tests, and
 *  the runtime posts notify.push over an already-paired NodeSession. */
object Wire {
    fun writeLen(out: OutputStream, payload: ByteArray) {
        DataOutputStream(out).writeInt(payload.size)
        out.write(payload)
        out.flush()
    }

    fun readLen(input: InputStream, max: Int = 64 * 1024): ByteArray {
        val dis = DataInputStream(input)
        val n = dis.readInt()
        require(n in 0..max) { "frame too large" }
        return dis.readNBytes(n)
    }

    fun helloJson(deviceId: String, idPk: ByteArray, nPk: ByteArray, name: String): JSONObject {
        return JSONObject()
            .put("proto", 1)
            .put("device_id", deviceId)
            .put("id_pk", Base64.encodeToString(idPk, Base64.NO_WRAP))
            .put("n_pk", Base64.encodeToString(nPk, Base64.NO_WRAP))
            .put("caps", JSONArray().put("notify").put("clip").put("file"))
            .put("name", name)
            .put("platform", "android")
    }

    fun notifyPush(
        uid: String,
        appId: String,
        appName: String,
        title: String,
        body: String,
        ts: Long,
    ): JSONObject {
        return JSONObject()
            .put("uid", uid)
            .put("app_id", appId)
            .put("app_name", appName)
            .put("title", title.take(4096))
            .put("body", body.take(4096))
            .put("ts", ts)
            .put("actions", JSONArray().put("copy").put("dismiss"))
    }

    fun sha256(bytes: ByteArray): ByteArray =
        MessageDigest.getInstance("SHA-256").digest(bytes)

    fun deviceIdFromIdPk(idPk: ByteArray): String {
        val d = sha256(idPk).copyOfRange(0, 10)
        return "tdev_" + base32Lower(d)
    }

    private fun base32Lower(bytes: ByteArray): String {
        val alphabet = "abcdefghijklmnopqrstuvwxyz234567"
        val out = StringBuilder()
        var buffer = 0
        var bits = 0
        for (b in bytes) {
            buffer = (buffer shl 8) or (b.toInt() and 0xff)
            bits += 8
            while (bits >= 5) {
                bits -= 5
                out.append(alphabet[(buffer shr bits) and 31])
            }
        }
        if (bits > 0) {
            out.append(alphabet[(buffer shl (5 - bits)) and 31])
        }
        return out.toString()
    }

    fun randomKey(): ByteArray {
        val b = ByteArray(32)
        SecureRandom().nextBytes(b)
        return b
    }
}
