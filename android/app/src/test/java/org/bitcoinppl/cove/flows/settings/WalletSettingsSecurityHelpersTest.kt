package org.bitcoinppl.cove.flows.settings

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

class WalletSettingsSecurityHelpersTest {
    @Test
    fun deletionFlowStopsAfterTheRequiredConfirmationCount() {
        assertNull(deletionFlow(requiredConfirmations = 1u).advance())

        val second = deletionFlow(requiredConfirmations = 2u).advance()
        assertEquals("Confirm Deletion", second?.title)
        assertNull(second?.advance())

        val third = deletionFlow(requiredConfirmations = 3u).advance()?.advance()
        assertEquals("Final Warning", third?.title)
        assertNull(third?.advance())
    }

    private fun deletionFlow(requiredConfirmations: UByte): WalletDeletionFlow =
        WalletDeletionFlow.start(
            walletName = "Test Wallet",
            firstMessage = "First warning",
            requiredConfirmations = requiredConfirmations,
        )
}
