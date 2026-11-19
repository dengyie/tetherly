package app.tetherly

import android.content.Context
import java.io.File

/**
 * Android Phase 1 session: hand the persisted identity + target address to the
 * native layer, which owns the TCP/Noise loop and reconnects internally. This
 * class only bridges lifecycle and exposes device_id + connection state.
 *
 * The identity is a real Ed25519 keypair generated and persisted by Rust in
 * app-private storage (`filesDir/tetherly/identity.bin`); Kotlin never handles
 * secret bytes, and never opens a raw TCP itself.
 */
class NodeSession(ctx: Context) {
    private val app = ctx.applicationContext
    private val prefs = Prefs(app)
    private val dataDir = File(app.filesDir, "tetherly").also { it.mkdirs() }

    @Volatile
    private var deviceId: String = ""

    @Volatile
    private var started: Boolean = false

    /** Local device_id, or "unavailable" before the first successful attach. */
    val identity: String
        get() = if (deviceId.isEmpty()) "unavailable" else deviceId

    /**
     * True once the native layer has persisted an identity for this install:
     * a restart can then re-attach without the first-time PIN.
     */
    fun pairedBefore(): Boolean = AutoConnectPolicy.identityPersisted(dataDir)

    /**
     * attach() replaces any previous session internally, but a redundant
     * attach still tears down and redials; callers that may race the
     * listener/foreground path use this idempotent form instead.
     */
    fun ensureStarted() {
        if (started) return
        start()
    }

    fun start() {
        if (!NativeBridge.ensureLoaded()) return
        val addr = NativeBridge.splitAddr(prefs.desktopAddr) ?: return
        // attach() replaces any previous session internally (the old handle is
        // stopped). device_id is stable across restarts.
        val err = NativeBridge.attach(dataDir.absolutePath, addr.first, addr.second, prefs.pin)
        if (err.isEmpty()) {
            deviceId = NativeBridge.deviceId()
            started = true
        }
    }

    fun stop() {
        started = false
        NativeBridge.stop()
    }

    /** Cold-start gate: reconnect automatically only after a prior pairing. */
    fun shouldAutoStart(): Boolean =
        AutoConnectPolicy.shouldAutoStart(prefs.desktopAddr, pairedBefore())

    fun state(): Int = NativeBridge.state()

    fun lastError(): String = NativeBridge.lastError()

    fun enqueueNotify(n: PhoneNotify) {
        NativeBridge.sendNotify(n)
    }

    /** Quick replies typed on the desktop, drained exactly once. */
    fun takeReplies(): List<PhoneReply> = NativeBridge.takeReplies()
}
