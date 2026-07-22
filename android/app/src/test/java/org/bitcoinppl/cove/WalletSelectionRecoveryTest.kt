package org.bitcoinppl.cove

import org.bitcoinppl.cove_core.WalletManagerException
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertSame
import org.junit.Assert.assertTrue
import org.junit.Test
import kotlin.coroutines.cancellation.CancellationException

class WalletSelectionRecoveryTest {
    @Test
    fun recoveryTriesRequestedThenCachedThenWalletDisplayOrder() {
        val recovery =
            WalletTransitionRecovery.create(
                requestedId = "wallet-b",
                cachedId = "wallet-a",
                displayedIds = listOf("wallet-c", "wallet-a", "wallet-b", "wallet-d"),
            )

        assertEquals("wallet-b", recovery.nextCandidate())
        assertEquals("wallet-a", recovery.nextCandidate())
        assertEquals("wallet-c", recovery.nextCandidate())
        assertEquals("wallet-d", recovery.nextCandidate())
        assertEquals(null, recovery.nextCandidate())
    }

    @Test
    fun recoveryExhaustionDoesNotRetryAttemptedWallets() {
        val recovery =
            WalletTransitionRecovery.create(
                requestedId = "wallet-a",
                cachedId = "wallet-a",
                displayedIds = listOf("wallet-a", "wallet-a"),
            )

        assertEquals("wallet-a", recovery.nextCandidate())
        assertEquals(null, recovery.nextCandidate())
    }

    @Test
    fun completedBootstrapUsesMatchingCacheEvenWhenGenerationChanged() {
        val initialState = WalletManagerCacheState()
        val decision =
            WalletManagerBootstrapDecision.resolve(
                loadToken = initialState.loadToken("wallet-b"),
                cacheState = initialState.managerChanged(),
                cachedWalletId = "wallet-b",
            )

        assertEquals(WalletManagerBootstrapDecision.UseCached, decision)
    }

    @Test
    fun completedBootstrapCancelsForSupersedingReplacement() {
        val initialState = WalletManagerCacheState()
        val decision =
            WalletManagerBootstrapDecision.resolve(
                loadToken = initialState.loadToken("wallet-b"),
                cacheState = initialState.managerChanged(),
                cachedWalletId = "wallet-c",
            )

        assertEquals(WalletManagerBootstrapDecision.Cancel, decision)
    }

    @Test
    fun completedBootstrapInstallsAfterUnrelatedCacheClear() {
        val initialState = WalletManagerCacheState()
        val decision =
            WalletManagerBootstrapDecision.resolve(
                loadToken = initialState.loadToken("wallet-b"),
                cacheState = initialState.managerChanged(),
                cachedWalletId = null,
            )

        assertEquals(WalletManagerBootstrapDecision.Install, decision)
    }

    @Test
    fun completedBootstrapInstallsWhenCacheIsUnchanged() {
        val initialState = WalletManagerCacheState()
        val decision =
            WalletManagerBootstrapDecision.resolve(
                loadToken = initialState.loadToken("wallet-b"),
                cacheState = initialState,
                cachedWalletId = "wallet-a",
            )

        assertEquals(WalletManagerBootstrapDecision.Install, decision)
    }

    @Test
    fun completedBootstrapCancelsAfterAllInvalidationWithEmptyCache() {
        val initialState = WalletManagerCacheState()
        val decision =
            WalletManagerBootstrapDecision.resolve(
                loadToken = initialState.loadToken("wallet-b"),
                cacheState = initialState.invalidate(WalletManagerInvalidation.All),
                cachedWalletId = null,
            )

        assertEquals(WalletManagerBootstrapDecision.Cancel, decision)
    }

    @Test
    fun completedBootstrapCancelsAfterTargetInvalidationWithEmptyCache() {
        val initialState = WalletManagerCacheState()
        val decision =
            WalletManagerBootstrapDecision.resolve(
                loadToken = initialState.loadToken("wallet-b"),
                cacheState =
                    initialState.invalidate(WalletManagerInvalidation.Wallet("wallet-b")),
                cachedWalletId = null,
            )

        assertEquals(WalletManagerBootstrapDecision.Cancel, decision)
    }

    @Test
    fun completedBootstrapInstallsAfterUnrelatedTargetInvalidationWithEmptyCache() {
        val initialState = WalletManagerCacheState()
        val decision =
            WalletManagerBootstrapDecision.resolve(
                loadToken = initialState.loadToken("wallet-b"),
                cacheState =
                    initialState.invalidate(WalletManagerInvalidation.Wallet("wallet-a")),
                cachedWalletId = null,
            )

        assertEquals(WalletManagerBootstrapDecision.Install, decision)
    }

    @Test
    fun matchingCacheWinsAfterLoadWasInvalidated() {
        val initialState = WalletManagerCacheState()
        val currentState =
            initialState
                .invalidate(WalletManagerInvalidation.All)
                .invalidate(WalletManagerInvalidation.Wallet("wallet-b"))
                .managerChanged()
        val decision =
            WalletManagerBootstrapDecision.resolve(
                loadToken = initialState.loadToken("wallet-b"),
                cacheState = currentState,
                cachedWalletId = "wallet-b",
            )

        assertEquals(WalletManagerBootstrapDecision.UseCached, decision)
    }

    @Test
    fun repeatedInvalidationAdvancesWithoutManagerChanges() {
        val targetedState =
            WalletManagerCacheState()
                .invalidate(WalletManagerInvalidation.Wallet("wallet-b"))
        val targetedToken = targetedState.loadToken("wallet-b")
        val retargetedState =
            targetedState.invalidate(WalletManagerInvalidation.Wallet("wallet-b"))

        assertTrue(retargetedState.invalidated(targetedToken))

        val allState = retargetedState.invalidate(WalletManagerInvalidation.All)
        val allToken = allState.loadToken("wallet-c")
        val reinvalidatedAllState = allState.invalidate(WalletManagerInvalidation.All)

        assertTrue(reinvalidatedAllState.invalidated(allToken))
    }

    @Test
    fun missingWalletFailureAllowsFallback() {
        val disposition =
            WalletPreparationFailureDisposition.classify(
                WalletManagerException.WalletDoesNotExist(),
            )

        assertEquals(WalletPreparationFailureDisposition.MissingWallet, disposition)
    }

    @Test
    fun databaseCorruptionFailureAllowsFallbackAndRetainsDetails() {
        val error = WalletManagerException.DatabaseCorruption("wallet-b", "corrupt")
        val disposition =
            WalletPreparationFailureDisposition.classify(error)
                as WalletPreparationFailureDisposition.CorruptedWallet

        assertSame(error, disposition.error)
    }

    @Test
    fun ordinaryWalletPreparationFailureIsClassifiedForRethrow() {
        val error = WalletManagerException.GetSelectedWalletException("failed")
        val disposition =
            WalletPreparationFailureDisposition.classify(error)
                as WalletPreparationFailureDisposition.Rethrow

        assertSame(error, disposition.error)
    }

    @Test
    fun walletPreparationCancellationIsRethrown() {
        val error = CancellationException("cancelled")
        val disposition =
            WalletPreparationFailureDisposition.classify(error)
                as WalletPreparationFailureDisposition.Rethrow

        assertSame(error, disposition.error)
    }

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
