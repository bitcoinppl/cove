package org.bitcoinppl.cove

import kotlin.coroutines.cancellation.CancellationException
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertSame
import org.junit.Assert.assertTrue
import org.junit.Test

class WalletSelectionRecoveryTest {
    @Test
    fun recoversWalletSelectionWithoutPoppingRoute() {
        var didPopRoute = false

        val result =
            recoverWalletSelectionOrPopRoute(
                selectLatestOrNewWallet = {},
                popRoute = {
                    didPopRoute = true
                    RoutePopResult.Popped
                },
            )

        assertEquals(WalletSelectionRecoveryResult.Recovered, result)
        assertFalse(didPopRoute)
    }

    @Test
    fun popsRouteWhenWalletSelectionRecoveryFails() {
        val recoveryError = IllegalStateException("wallet selection failed")
        var didPopRoute = false

        val result =
            recoverWalletSelectionOrPopRoute(
                selectLatestOrNewWallet = { throw recoveryError },
                popRoute = {
                    didPopRoute = true
                    RoutePopResult.Popped
                },
            )

        assertTrue(didPopRoute)
        assertSame(recoveryError, (result as WalletSelectionRecoveryResult.PoppedRoute).recoveryError)
    }

    @Test
    fun reportsNoRouteToPopWhenRecoveryFailsAndPopIsNoop() {
        val recoveryError = IllegalStateException("wallet selection failed")

        val result =
            recoverWalletSelectionOrPopRoute(
                selectLatestOrNewWallet = { throw recoveryError },
                popRoute = { RoutePopResult.NoRouteToPop },
            ) as WalletSelectionRecoveryResult.NoRouteToPop

        assertSame(recoveryError, result.recoveryError)
    }

    @Test
    fun reportsNavigationFailureWhenRoutePopFailsWithResult() {
        val recoveryError = IllegalStateException("wallet selection failed")
        val navigationError = IllegalStateException("pop route failed")

        val result =
            recoverWalletSelectionOrPopRoute(
                selectLatestOrNewWallet = { throw recoveryError },
                popRoute = { RoutePopResult.Failed(navigationError) },
            ) as WalletSelectionRecoveryResult.FailedToPopRoute

        assertSame(recoveryError, result.recoveryError)
        assertSame(navigationError, result.navigationError)
    }

    @Test
    fun reportsNavigationFailureWhenRecoveryAndRoutePopFail() {
        val recoveryError = IllegalStateException("wallet selection failed")
        val navigationError = IllegalStateException("pop route failed")

        val result =
            recoverWalletSelectionOrPopRoute(
                selectLatestOrNewWallet = { throw recoveryError },
                popRoute = { throw navigationError },
            ) as WalletSelectionRecoveryResult.FailedToPopRoute

        assertSame(recoveryError, result.recoveryError)
        assertSame(navigationError, result.navigationError)
    }

    @Test(expected = CancellationException::class)
    fun rethrowsWalletSelectionCancellationWithoutPoppingRoute() {
        var didPopRoute = false

        try {
            recoverWalletSelectionOrPopRoute(
                selectLatestOrNewWallet = { throw CancellationException("cancelled") },
                popRoute = {
                    didPopRoute = true
                    RoutePopResult.Popped
                },
            )
        } finally {
            assertFalse(didPopRoute)
        }
    }

    @Test(expected = CancellationException::class)
    fun rethrowsRoutePopCancellation() {
        recoverWalletSelectionOrPopRoute(
            selectLatestOrNewWallet = { throw IllegalStateException("wallet selection failed") },
            popRoute = { throw CancellationException("cancelled") },
        )
    }
}
