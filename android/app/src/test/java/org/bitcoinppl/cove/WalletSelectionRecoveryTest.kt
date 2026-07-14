package org.bitcoinppl.cove

import kotlin.coroutines.cancellation.CancellationException
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertSame
import org.junit.Assert.assertTrue
import org.junit.Assert.fail
import org.junit.Test

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
    fun atomicCacheCreationFailureLeavesCachedManagerInstalled() {
        val cached = TestWalletManager("wallet-a")
        var installed: TestWalletManager? = cached

        try {
            getOrCreateWalletManagerAtomically(
                cachedManager = cached,
                requestedId = "wallet-b",
                id = TestWalletManager::id,
                create = { throw IllegalStateException("wallet-b failed") },
                install = {
                    installed = it
                    it
                },
            )
            fail("Expected wallet creation to fail")
        } catch (_: IllegalStateException) {
            assertSame(cached, installed)
        }
    }

    @Test
    fun atomicCacheInstallsCandidateOnlyAfterCreationSucceeds() {
        val events = mutableListOf<String>()
        val candidate = TestWalletManager("wallet-b")

        val result =
            getOrCreateWalletManagerAtomically(
                cachedManager = TestWalletManager("wallet-a"),
                requestedId = candidate.id,
                id = TestWalletManager::id,
                create = {
                    events += "created"
                    candidate
                },
                install = {
                    events += "installed"
                    it
                },
            )

        assertSame(candidate, result)
        assertEquals(listOf("created", "installed"), events)
    }

    @Test
    fun atomicCacheReusesMatchingManagerWithoutCreation() {
        val cached = TestWalletManager("wallet-a")

        val result =
            getOrCreateWalletManagerAtomically(
                cachedManager = cached,
                requestedId = cached.id,
                id = TestWalletManager::id,
                create = { error("must not create") },
                install = { error("must not install") },
            )

        assertSame(cached, result)
    }

    @Test
    fun completedBootstrapUsesMatchingCacheEvenWhenGenerationChanged() {
        val decision =
            WalletManagerBootstrapDecision.resolve(
                targetId = "wallet-b",
                capturedGeneration = 1,
                currentGeneration = 2,
                cachedWalletId = "wallet-b",
            )

        assertEquals(WalletManagerBootstrapDecision.UseCached, decision)
    }

    @Test
    fun completedBootstrapCancelsWhenAnotherWalletChangedTheCache() {
        val decision =
            WalletManagerBootstrapDecision.resolve(
                targetId = "wallet-b",
                capturedGeneration = 1,
                currentGeneration = 2,
                cachedWalletId = "wallet-c",
            )

        assertEquals(WalletManagerBootstrapDecision.Cancel, decision)
    }

    @Test
    fun completedBootstrapInstallsWhenCacheIsUnchanged() {
        val decision =
            WalletManagerBootstrapDecision.resolve(
                targetId = "wallet-b",
                capturedGeneration = 1,
                currentGeneration = 1,
                cachedWalletId = "wallet-a",
            )

        assertEquals(WalletManagerBootstrapDecision.Install, decision)
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

private data class TestWalletManager(
    val id: String,
)
