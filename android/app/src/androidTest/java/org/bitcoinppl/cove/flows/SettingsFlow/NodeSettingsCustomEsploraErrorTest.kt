package org.bitcoinppl.cove.flows.SettingsFlow

import android.view.View
import androidx.compose.ui.platform.LocalView
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.semantics.getOrNull
import androidx.compose.ui.test.hasSetTextAction
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performScrollTo
import androidx.compose.ui.test.performTextReplacement
import androidx.core.view.ViewCompat
import androidx.core.view.WindowInsetsCompat
import org.bitcoinppl.cove.AppManager
import org.bitcoinppl.cove.test.bootstrapRustRuntimeForUiTest
import org.bitcoinppl.cove.test.coveComposeRule
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
    val compose = coveComposeRule()

    private lateinit var app: AppManager
    private lateinit var initialNetwork: Network
    private lateinit var hostView: View

    @Before
    fun hostScreenOnSignet() {
        bootstrapRustRuntimeForUiTest()

        app = AppManager.getInstance()
        initialNetwork = app.selectedNetwork
        changeNetwork(Network.SIGNET)

        compose.setContent {
            hostView = LocalView.current
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

        // CI's slower emulator has the keyboard fully up by the time the test taps Save, so wait
        // for it on every machine. The local network section sits above the form, so the keyboard
        // covers Save until it is scrolled into view, as a person would
        compose.waitUntil(timeoutMillis = 10_000) { isKeyboardShown() }
        compose.onNodeWithText("Save Custom Node").performScrollTo().performClick()

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

    private fun isKeyboardShown(): Boolean =
        compose.runOnUiThread {
            ViewCompat.getRootWindowInsets(hostView)?.isVisible(WindowInsetsCompat.Type.ime()) == true
        }

    private fun changeNetwork(network: Network) {
        compose.runOnUiThread { app.dispatch(AppAction.ChangeNetwork(network)) }
        compose.waitUntil(timeoutMillis = 5_000) { app.selectedNetwork == network }
    }

    private companion object {
        const val UNREACHABLE_ESPLORA_URL = "https://127.0.0.1/api"
    }
}
