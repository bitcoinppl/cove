package org.bitcoinppl.cove.sheets

import org.bitcoinppl.cove_core.SendFlowException
import org.bitcoinppl.cove_core.WalletManagerException
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
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

    @Test
    fun nonNumericCustomFeeRateIsRejectedInsteadOfFallingBackToTheSelectedRate() {
        for (text in listOf("abc", "", " ", "NaN", "Infinity", "1.2.3")) {
            assertNull("\"$text\" should not parse as a fee rate", parseCustomFeeRate(text))
        }
    }

    @Test
    fun numericCustomFeeRateParsesToHundredths() {
        assertEquals(2.35f, parseCustomFeeRate(" 2.349 ")!!, 0f)
        assertEquals(0f, parseCustomFeeRate("0")!!, 0f)
    }
}
