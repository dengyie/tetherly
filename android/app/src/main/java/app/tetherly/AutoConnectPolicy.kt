package app.tetherly

import java.io.File

/**
 * Cold-start auto-reconnect gate. A paired install (native identity persisted
 * plus a stored desktop address) reconnects on process start without user
 * input; a fresh install keeps waiting for the explicit 保存并连接 so an
 * unconfigured app never dials a default address on its own.
 */
object AutoConnectPolicy {
    fun shouldAutoStart(desktopAddr: String, identityPersisted: Boolean): Boolean =
        desktopAddr.isNotBlank() && identityPersisted

    /** The identity marker the native layer persists under the data dir. */
    fun identityPersisted(dataDir: File): Boolean =
        File(dataDir, "identity.bin").isFile
}
