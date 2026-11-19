package app.tetherly

import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.rules.TemporaryFolder

/**
 * Cold-start auto-reconnect gate: only a paired install (persisted identity
 * plus a stored desktop address) reconnects without user input.
 */
class AutoConnectPolicyTest {
    @get:Rule
    val tmp = TemporaryFolder()

    @Test
    fun `paired install with a stored address auto-starts`() {
        val dir = tmp.newFolder()
        assertTrue(java.io.File(dir, "identity.bin").createNewFile())
        assertTrue(AutoConnectPolicy.shouldAutoStart("127.0.0.1:45717", AutoConnectPolicy.identityPersisted(dir)))
    }

    @Test
    fun `fresh install without identity never auto-starts`() {
        val dir = tmp.newFolder()
        assertFalse(AutoConnectPolicy.identityPersisted(dir))
        assertFalse(AutoConnectPolicy.shouldAutoStart("127.0.0.1:45717", false))
    }

    @Test
    fun `blank address blocks auto-start even when paired`() {
        val dir = tmp.newFolder()
        assertTrue(java.io.File(dir, "identity.bin").createNewFile())
        assertFalse(AutoConnectPolicy.shouldAutoStart("", true))
        assertFalse(AutoConnectPolicy.shouldAutoStart("   ", true))
    }
}
