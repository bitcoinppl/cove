package org.bitcoinppl.cove.nfc

import kotlin.coroutines.cancellation.CancellationException
import org.junit.Assert.assertFalse
import org.junit.Assert.assertThrows
import org.junit.Assert.assertTrue
import org.junit.Test

class NfcOperationGateTest {
    @Test
    fun cancellationAndNewScanRejectTheOldToken() {
        val gate = NfcOperationGate()
        val cancelledOwner = TapCardNfcManager.OperationToken()
        val nextOwner = TapCardNfcManager.OperationToken()
        val cancelled = gate.begin(cancelledOwner)

        gate.end(cancelled)
        val next = gate.begin(nextOwner)

        assertFalse(gate.isCurrent(cancelled))
        assertTrue(gate.isCurrent(next))

        gate.end(cancelled)
        assertTrue(gate.isCurrent(next))
    }

    @Test
    fun beginTwiceMakesOnlyTheNewestTokenCurrent() {
        val gate = NfcOperationGate()
        val first = gate.begin(TapCardNfcManager.OperationToken())
        val second = gate.begin(TapCardNfcManager.OperationToken())

        assertFalse(gate.isCurrent(first))
        assertTrue(gate.isCurrent(second))

        gate.end(first)
        assertTrue(gate.isCurrent(second))
    }

    @Test
    fun oldOwnerCannotCancelTheNewOwnersScan() {
        val gate = NfcOperationGate()
        val oldOwner = TapCardNfcManager.OperationToken()
        val newOwner = TapCardNfcManager.OperationToken()
        val oldSession = gate.begin(oldOwner)

        gate.end(oldSession)
        val newSession = gate.begin(newOwner)

        oldOwner.cancel()

        assertFalse(gate.cancel(oldOwner))
        assertTrue(gate.isCurrent(newSession))
        assertFalse(newOwner.isCancelled())
        assertTrue(gate.cancel(newOwner))
        assertFalse(gate.isCurrent(newSession))
    }

    @Test
    fun closedOwnerCannotStartAQueuedScan() {
        val owner = TapCardNfcManager.OperationToken()

        owner.cancel()

        assertThrows(CancellationException::class.java) {
            owner.ensureActive()
        }
    }
}
