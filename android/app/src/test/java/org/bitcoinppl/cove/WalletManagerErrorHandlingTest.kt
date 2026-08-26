package org.bitcoinppl.cove

import org.bitcoinppl.cove_core.LabelManagerException
import org.bitcoinppl.cove_core.WalletAddressType
import org.bitcoinppl.cove_core.WalletLifecycleFailure
import org.bitcoinppl.cove_core.WalletManagerException
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class WalletManagerErrorHandlingTest {
    @Test
    fun closingFailureRetriesOnlyTheRequestedWallet() {
        val error =
            WalletManagerException.WalletLifecycle(
                WalletLifecycleFailure.ManagerClosing("wallet-a"),
            )

        assertTrue(error.isWalletManagerClosing("wallet-a"))
        assertFalse(error.isWalletManagerClosing("wallet-b"))
    }

    @Test
    fun constructionInProgressRetriesOnlyTheRequestedWallet() {
        val error =
            WalletManagerException.WalletLifecycle(
                WalletLifecycleFailure.ConstructionInProgress("wallet-a"),
            )

        assertTrue(error.isWalletManagerConstructionInProgress("wallet-a"))
        assertFalse(error.isWalletManagerConstructionInProgress("wallet-b"))
        assertFalse(error.isWalletManagerClosing("wallet-a"))
    }

    @Test
    fun fallibleWalletReadUsesItsSafeDefault() {
        var reported = false
        val value =
            walletManagerValueOr(false, { reported = true }) {
                throw WalletManagerException.ManagerClosed()
            }

        assertFalse(value)
        assertTrue(reported)
    }

    @Test
    fun fallibleNullableWalletReadUsesItsNullFallback() {
        var reported = false
        val value: UInt? =
            walletManagerValueOr(null, { reported = true }) {
                throw WalletManagerException.ManagerClosed()
            }

        assertNull(value)
        assertTrue(reported)
    }

    @Test
    fun fallibleLabelReadUsesItsSafeDefault() {
        var reported = false
        val value =
            walletLabelValueOr(false, { reported = true }) {
                throw LabelManagerException.ManagerClosed()
            }

        assertFalse(value)
        assertTrue(reported)
    }

    @Test
    fun committedSwitchRecoveryIsNotAPreCommitFailure() {
        val error =
            WalletManagerException.AddressTypeSwitchCommittedWithRecoveryPending(
                WalletAddressType.LEGACY,
                emptyList(),
            )

        assertEquals(
            WalletAddressTypeSwitchResult.COMMITTED_WITH_RECOVERY_PENDING,
            error.committedAddressTypeSwitchResult(WalletAddressType.LEGACY),
        )
        assertNull(error.committedAddressTypeSwitchResult(WalletAddressType.WRAPPED_SEGWIT))

        val unableToSwitch =
            WalletManagerException.UnableToSwitch(
                WalletAddressType.LEGACY,
                "not committed",
            )

        assertNull(
            unableToSwitch.committedAddressTypeSwitchResult(WalletAddressType.LEGACY),
        )
    }
}
