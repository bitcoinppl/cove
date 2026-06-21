package org.bitcoinppl.cove.flows.SelectedWalletFlow

import org.bitcoinppl.cove.WalletLoadState
import org.bitcoinppl.cove.initialScanActive
import org.bitcoinppl.cove.initialScanComplete
import org.bitcoinppl.cove.initialScanIncomplete
import org.bitcoinppl.cove_core.InitialScanActivity
import org.bitcoinppl.cove_core.WalletLedgerState
import org.bitcoinppl.cove_core.WalletScanPhase
import org.bitcoinppl.cove_core.WalletScanStatus
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class SelectedWalletScreenHelpersTest {
    @Test
    fun completeLedgerStateMatchesInitialScanSemantics() {
        val state = WalletLedgerState.Complete

        assertTrue(state.initialScanComplete)
        assertFalse(state.initialScanIncomplete)
        assertFalse(state.initialScanActive)
    }

    @Test
    fun incompleteIdleLedgerStateMatchesInitialScanSemantics() {
        val state = WalletLedgerState.InitialScanIncomplete(InitialScanActivity.IDLE)

        assertFalse(state.initialScanComplete)
        assertTrue(state.initialScanIncomplete)
        assertFalse(state.initialScanActive)
    }

    @Test
    fun incompleteActiveLedgerStateMatchesInitialScanSemantics() {
        val state = WalletLedgerState.InitialScanIncomplete(InitialScanActivity.ACTIVE)

        assertFalse(state.initialScanComplete)
        assertTrue(state.initialScanIncomplete)
        assertTrue(state.initialScanActive)
    }

    @Test
    fun loadedWalletCanRefresh() {
        assertTrue(
            canRefreshSelectedWallet(
                WalletLoadState.LOADED(emptyList()),
                WalletScanStatus.Idle,
            ),
        )
    }

    @Test
    fun loadedWalletCannotRefreshDuringActiveScan() {
        assertFalse(
            canRefreshSelectedWallet(
                WalletLoadState.LOADED(emptyList()),
                WalletScanStatus.ScanningPendingProgress(WalletScanPhase.FULL),
            ),
        )
    }

    @Test
    fun completedInitialScanCanRefreshWhenLoadStateScanningAndScanStatusIdle() {
        // scanning load state can lag after the ledger reports initial scan complete
        assertTrue(
            canRefreshSelectedWallet(
                WalletLoadState.SCANNING(emptyList()),
                WalletScanStatus.Idle,
            ),
        )
    }

    @Test
    fun activeScanCannotRefresh() {
        assertFalse(
            canRefreshSelectedWallet(
                WalletLoadState.SCANNING(emptyList()),
                WalletScanStatus.ScanningPendingProgress(WalletScanPhase.FULL),
            ),
        )
    }

    @Test
    fun loadingWalletCannotRefresh() {
        assertFalse(
            canRefreshSelectedWallet(
                WalletLoadState.LOADING,
                WalletScanStatus.Idle,
            ),
        )
    }
}
