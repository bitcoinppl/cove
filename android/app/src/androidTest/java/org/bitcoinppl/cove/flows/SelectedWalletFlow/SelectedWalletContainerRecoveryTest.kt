package org.bitcoinppl.cove.flows.SelectedWalletFlow

import androidx.compose.ui.test.junit4.createComposeRule
import androidx.test.ext.junit.runners.AndroidJUnit4
import org.bitcoinppl.cove.AppManager
import org.bitcoinppl.cove.test.bootstrapRustRuntimeForUiTest
import org.bitcoinppl.cove.ui.theme.CoveTheme
import org.bitcoinppl.cove_core.Route
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

@RunWith(AndroidJUnit4::class)
class SelectedWalletContainerRecoveryTest {
    @get:Rule
    val compose = createComposeRule()

    @Before
    fun bootstrapRustRuntime() {
        bootstrapRustRuntimeForUiTest()
    }

    // regression for #916: a cold launch restored a wallet id the active network does not have,
    // and the container stayed on its loading spinner instead of leaving the stale route
    @Test
    fun staleSelectedWalletRouteFallsBackInsteadOfLoadingForever() {
        val app = AppManager.getInstance()
        val originalRoute = app.router.default
        val staleRoute = Route.SelectedWallet(STALE_WALLET_ID)

        // resetRoute reconciles asynchronously, so the stale route must be in place before the
        // container mounts, or leaving it would prove nothing
        compose.runOnUiThread { app.resetRoute(staleRoute) }
        compose.waitUntil(timeoutMillis = RECOVERY_TIMEOUT_MS) { app.router.default == staleRoute }

        try {
            compose.setContent {
                CoveTheme(dynamicColor = false) {
                    SelectedWalletContainer(app = app, id = STALE_WALLET_ID)
                }
            }

            compose.waitUntil(timeoutMillis = RECOVERY_TIMEOUT_MS) {
                app.router.default != staleRoute
            }

            assertTrue(
                "fell back to ${app.router.default}, not a wallet the app has or new wallet",
                isRecoveryRoute(app, app.router.default),
            )
        } finally {
            compose.runOnUiThread { app.resetRoute(originalRoute) }
        }
    }

    private fun isRecoveryRoute(
        app: AppManager,
        route: Route,
    ): Boolean =
        when (route) {
            is Route.NewWallet -> true
            is Route.SelectedWallet -> app.walletMetadata(route.v1) != null
            else -> false
        }

    private companion object {
        const val STALE_WALLET_ID = "wallet-from-another-network"
        const val RECOVERY_TIMEOUT_MS = 10_000L
    }
}
