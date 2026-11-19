package app.tetherly

import org.junit.Assert.assertFalse
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class ForegroundKeepStateMachineTest {
    @Test
    fun serviceTransitionsFromStartingToRunning() {
        val machine = ForegroundKeepStateMachine()
        assertEquals(ForegroundKeepState.STOPPED, machine.state)
        assertTrue(machine.markStarting())
        machine.markCreated()
        assertTrue(machine.markRunning())
        assertEquals(ForegroundKeepState.RUNNING, machine.state)
        assertFalse(machine.markStarting())
    }

    @Test
    fun timeoutIsObservableAndOnlyHandledOnce() {
        val machine = ForegroundKeepStateMachine()
        machine.markStarting()
        machine.markCreated()
        machine.markRunning()

        assertTrue(machine.markTimedOut())
        assertEquals(ForegroundKeepState.TIMED_OUT, machine.state)
        assertFalse(machine.markTimedOut())
        machine.markRestartPending()
        assertEquals(ForegroundKeepState.RESTART_PENDING, machine.state)
    }

    @Test
    fun explicitStopBlocksTimeoutRecovery() {
        val machine = ForegroundKeepStateMachine()
        machine.markStarting()
        machine.markStopped()

        assertFalse(machine.markTimedOut())
        assertEquals(ForegroundKeepState.STOPPED, machine.state)
    }
}
