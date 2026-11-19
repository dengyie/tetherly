package app.tetherly

import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * Locks the session-teardown ownership contract: only explicit user stop (and
 * the deliberate Android 15 timeout path, which immediately schedules a
 * restart) may end the native session. Transient service/listener churn from
 * the OS must NOT tear it down — on the .51/.52 cloud phones the ROM rebinds
 * the notification listener without notice, and nothing else would restart a
 * killed session until the next process cold start.
 */
class SessionOwnershipPolicyTest {
    @Test
    fun `transient OS churn never tears down the session`() {
        assertFalse(SessionOwnershipPolicy.teardownOnEvent(SessionOwnershipPolicy.Event.LISTENER_DISCONNECTED))
        assertFalse(SessionOwnershipPolicy.teardownOnEvent(SessionOwnershipPolicy.Event.LISTENER_DESTROYED))
        assertFalse(SessionOwnershipPolicy.teardownOnEvent(SessionOwnershipPolicy.Event.SERVICE_DESTROYED))
    }

    @Test
    fun `explicit stop tears down the session`() {
        assertTrue(SessionOwnershipPolicy.teardownOnEvent(SessionOwnershipPolicy.Event.EXPLICIT_STOP))
    }

    @Test
    fun `fgs timeout tears down because restart is scheduled`() {
        assertTrue(SessionOwnershipPolicy.teardownOnEvent(SessionOwnershipPolicy.Event.FGS_TIMEOUT))
    }
}
