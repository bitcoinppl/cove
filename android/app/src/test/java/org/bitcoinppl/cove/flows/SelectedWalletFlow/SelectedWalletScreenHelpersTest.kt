package org.bitcoinppl.cove.flows.SelectedWalletFlow

import org.bitcoinppl.cove.WalletLoadState
import org.bitcoinppl.cove_core.WalletScanPhase
import org.bitcoinppl.cove_core.WalletScanStatus
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class SelectedWalletScreenHelpersTest {
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
    fun idleIncompleteScanCanRefresh() {
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
