package org.bitcoinppl.cove.flows.SettingsFlow

import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.semantics.getOrNull
import androidx.compose.ui.test.hasSetTextAction
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performTextReplacement
import org.bitcoinppl.cove.AppManager
import org.bitcoinppl.cove.test.bootstrapRustRuntimeForUiTest
import org.bitcoinppl.cove.ui.theme.CoveTheme
import org.bitcoinppl.cove_core.AppAction
import org.bitcoinppl.cove_core.types.Network
import org.junit.After
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Rule
import org.junit.Test

/**
 * A Custom Esplora node that can't be reached reports the URL the person entered
 *
 * Before the fix the Esplora URL went through the Electrum rules, so `https://host/api` became
 * `ssl://host:50002/api` and the alert named a node nobody typed
 */
class NodeSettingsCustomEsploraErrorTest {
    @get:Rule
    val compose = createComposeRule()

    private lateinit var app: AppManager
    private lateinit var initialNetwork: Network

    @Before
    fun hostScreenOnSignet() {
        bootstrapRustRuntimeForUiTest()

        app = AppManager.getInstance()
        initialNetwork = app.selectedNetwork
        changeNetwork(Network.SIGNET)

        compose.setContent {
            CoveTheme {
                NodeSettingsScreen(app = app)
            }
        }
    }

    @After
    fun restoreNetwork() {
        changeNetwork(initialNetwork)
    }

    @Test
    fun unreachableCustomEsploraErrorShowsTheEnteredUrl() {
        compose.onNodeWithText("Custom Esplora").performClick()
        compose
            .onNode(hasSetTextAction() and hasText("URL"))
            .performTextReplacement(UNREACHABLE_ESPLORA_URL)
        compose.onNodeWithText("Save Custom Node").performClick()

        // nothing listens on the emulator's own https port, so the check fails fast
        compose.waitUntil(timeoutMillis = 30_000) {
            compose.onAllNodes(hasText("Connection Failed")).fetchSemanticsNodes().isNotEmpty()
        }

        val message =
            compose
                .onNode(hasText("Failed to connect to node", substring = true))
                .fetchSemanticsNode()
                .config
                .getOrNull(SemanticsProperties.Text)
                .orEmpty()
                .joinToString(" ")

        assertTrue("the alert should name $UNREACHABLE_ESPLORA_URL, got: $message", UNREACHABLE_ESPLORA_URL in message)
        assertFalse("the alert should not rewrite the URL as Electrum, got: $message", "ssl://" in message)
        assertFalse("the alert should not add the Electrum port, got: $message", ":50002" in message)
    }

    private fun changeNetwork(network: Network) {
        compose.runOnUiThread { app.dispatch(AppAction.ChangeNetwork(network)) }
        compose.waitUntil(timeoutMillis = 5_000) { app.selectedNetwork == network }
    }

    private companion object {
        const val UNREACHABLE_ESPLORA_URL = "https://127.0.0.1/api"
    }
}
