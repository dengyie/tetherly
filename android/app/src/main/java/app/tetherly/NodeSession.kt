package app.tetherly

import android.content.Context
import android.util.Log
import java.io.File
import java.net.InetSocketAddress
import java.net.Socket
import java.util.concurrent.Executors
import java.util.concurrent.atomic.AtomicBoolean

/**
 * Android Phase 1 session: persist identity, dial a desktop, send notify.push.
 * SPAKE2 / Noise run in `libtetherly_android.so` (cargo-ndk). Without the
 * library the listener still captures notifications but does not open a raw
 * TCP (that would fail the desktop handshake). Loopback tests on desktop
 * simulate this path.
 */
class NodeSession(ctx: Context) {
    private val app = ctx.applicationContext
    private val prefs = Prefs(app)
    private val exec = Executors.newSingleThreadExecutor()
    private val running = AtomicBoolean(false)
    val identity: PhoneIdentity = PhoneIdentity.loadOrCreate(File(app.filesDir, "identity.bin"))

    fun start() {
        if (!running.compareAndSet(false, true)) return
        exec.execute {
            while (running.get()) {
                try {
                    connectOnce()
                } catch (_: Exception) {
                    Log.i(TAG, "session retry")
                }
                try {
                    Thread.sleep(2_000)
                } catch (_: InterruptedException) {
                    return@execute
                }
            }
        }
    }

    fun stop() {
        running.set(false)
    }

    private fun connectOnce() {
        NativeBridge.ensureLoaded()
        if (!NativeBridge.isLoaded) {
            return
        }
        val addr = prefs.desktopAddr
        val parts = addr.split(":")
        if (parts.size != 2) return
        val host = parts[0]
        val port = parts[1].toIntOrNull() ?: return
        Socket().use { s ->
            s.connect(InetSocketAddress(host, port), 5_000)
            NativeBridge.attach(s, identity, prefs.pin)
            while (running.get() && s.isConnected) {
                Thread.sleep(1_000)
            }
        }
    }

    fun enqueueNotify(n: PhoneNotify) {
        exec.execute {
            try {
                NativeBridge.sendNotify(n)
            } catch (_: Exception) {
                Log.i(TAG, "notify dropped uid=${n.uid} app=${n.appId}")
            }
        }
    }

    companion object {
        private const val TAG = "tetherly"
    }
}

data class PhoneNotify(
    val uid: String,
    val appId: String,
    val appName: String,
    val title: String,
    val body: String,
    val ts: Long,
)

class PhoneIdentity(
    val idSk: ByteArray,
    val nSk: ByteArray,
) {
    val idPk: ByteArray get() = Wire.sha256(idSk)
    val deviceId: String get() = Wire.deviceIdFromIdPk(idPk)

    fun persist(file: File) {
        file.writeBytes(idSk + nSk)
    }

    companion object {
        fun loadOrCreate(file: File): PhoneIdentity {
            if (file.exists() && file.length() == 64L) {
                val b = file.readBytes()
                return PhoneIdentity(b.copyOfRange(0, 32), b.copyOfRange(32, 64))
            }
            val id = PhoneIdentity(Wire.randomKey(), Wire.randomKey())
            file.parentFile?.mkdirs()
            id.persist(file)
            return id
        }
    }
}

object NativeBridge {
    @Volatile
    var isLoaded: Boolean = false
        private set

    fun ensureLoaded() {
        if (isLoaded) return
        try {
            System.loadLibrary("tetherly_android")
            isLoaded = true
        } catch (_: UnsatisfiedLinkError) {
            isLoaded = false
        }
    }

    fun attach(socket: Socket, identity: PhoneIdentity, pin: String) {
        if (isLoaded) {
            nativeAttach(socket, identity.idSk, identity.nSk, pin)
        }
    }

    fun sendNotify(n: PhoneNotify) {
        if (isLoaded) {
            nativeSendNotify(n.uid, n.appId, n.appName, n.title, n.body, n.ts)
        }
    }

    private external fun nativeAttach(socket: Socket, idSk: ByteArray, nSk: ByteArray, pin: String)
    private external fun nativeSendNotify(
        uid: String,
        appId: String,
        appName: String,
        title: String,
        body: String,
        ts: Long,
    )
}
