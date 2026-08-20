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
    fun beginTwiceMakesOnlyTheNewestTokenCurrent() {
        val gate = NfcOperationGate()
        val first = gate.begin()
        val second = gate.begin()

        assertFalse(gate.isCurrent(first))
        assertTrue(gate.isCurrent(second))

        gate.end(first)
        assertTrue(gate.isCurrent(second))
    }
}
