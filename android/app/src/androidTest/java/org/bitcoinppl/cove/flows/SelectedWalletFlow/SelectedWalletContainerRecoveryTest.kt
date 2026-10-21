package org.bitcoinppl.cove.flows.SelectedWalletFlow

import androidx.compose.ui.test.junit4.createComposeRule
import androidx.test.ext.junit.runners.AndroidJUnit4
import org.bitcoinppl.cove.AppManager
import org.bitcoinppl.cove.test.bootstrapRustRuntimeForUiTest
import org.bitcoinppl.cove.ui.theme.CoveTheme
import org.bitcoinppl.cove_core.Route
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
        val staleRoute = Route.SelectedWallet(STALE_WALLET_ID)
        compose.runOnUiThread { app.resetRoute(staleRoute) }

        compose.setContent {
            CoveTheme(dynamicColor = false) {
                SelectedWalletContainer(app = app, id = STALE_WALLET_ID)
            }
        }

        compose.waitUntil(timeoutMillis = RECOVERY_TIMEOUT_MS) {
            app.router.default != staleRoute
        }
    }

    private companion object {
        const val STALE_WALLET_ID = "wallet-from-another-network"
        const val RECOVERY_TIMEOUT_MS = 10_000L
    }
}
