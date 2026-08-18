package org.bitcoinppl.cove.nfc

import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class NfcOperationGateTest {
    @Test
    fun cancellationAndNewScanRejectTheOldToken() {
        val gate = NfcOperationGate()
        val cancelled = gate.begin()

        gate.end(cancelled)
        val next = gate.begin()

        assertFalse(gate.isCurrent(cancelled))
        assertTrue(gate.isCurrent(next))

        gate.end(cancelled)
        assertTrue(gate.isCurrent(next))
    }

    @Test
    fun staleDelayedDisableOwnerCannotMatchTheNextScan() {
        val gate = NfcOperationGate()
        val oldToken = gate.begin()
        gate.end(oldToken)
        val newToken = gate.begin()

        assertFalse(gate.isCurrent(oldToken))
        assertTrue(gate.isCurrent(newToken))
    }
}
