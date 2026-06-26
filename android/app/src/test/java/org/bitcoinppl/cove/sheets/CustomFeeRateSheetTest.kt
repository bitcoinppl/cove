package org.bitcoinppl.cove.sheets

import org.bitcoinppl.cove_core.SendFlowException
import org.bitcoinppl.cove_core.WalletManagerException
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class CustomFeeRateSheetTest {
    @Test
    fun topLevelInsufficientFundsIsTooHighCustomFeeError() {
        assertTrue(isTooHighCustomFeeError(SendFlowException.InsufficientFunds()))
    }

    @Test
    fun wrappedWalletInsufficientFundsIsTooHighCustomFeeError() {
        val error =
            SendFlowException.WalletManager(
                WalletManagerException.InsufficientFunds("not enough funds"),
            )

        assertTrue(isTooHighCustomFeeError(error))
    }

    @Test
    fun otherWalletErrorsAreNotTooHighCustomFeeErrors() {
        val error =
            SendFlowException.WalletManager(
                WalletManagerException.LockedOutputsSelected(),
            )

        assertFalse(isTooHighCustomFeeError(error))
    }
}
