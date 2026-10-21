package org.bitcoinppl.cove.flows.SettingsFlow

import androidx.compose.ui.test.assertCountEquals
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onAllNodesWithContentDescription
import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.test.platform.app.InstrumentationRegistry
import androidx.test.uiautomator.UiDevice
import org.bitcoinppl.cove.AppManager
import org.bitcoinppl.cove.test.bootstrapRustRuntimeForUiTest
import org.bitcoinppl.cove.ui.theme.CoveTheme
import org.bitcoinppl.cove_core.types.Network
import org.bitcoinppl.cove_core.types.allNetworks
import org.junit.Assert.assertEquals
import org.junit.Before
import org.junit.Rule
import org.junit.Test

class NetworkSettingsScreenTest {
    @get:Rule
    val compose = createComposeRule()

    private lateinit var app: AppManager
    private lateinit var initialNetwork: Network
    private lateinit var otherNetwork: Network

    @Before
    fun hostScreen() {
        bootstrapRustRuntimeForUiTest()

        app = AppManager.getInstance()
        initialNetwork = app.selectedNetwork
        otherNetwork = allNetworks().first { it != initialNetwork }

        compose.setContent {
            CoveTheme {
                NetworkSettingsScreen(app = app)
            }
        }

        // the confirmation dialog only appears after choosing a different network
        compose.onNodeWithText(otherNetwork.toString()).performClick()
        compose.onNodeWithText("Warning: Network Changed").assertIsDisplayed()
    }

    @Test
    fun cancelButtonDismissesTheDialogAndKeepsTheNetwork() {
        compose.onNodeWithText("Cancel").performClick()

        assertNetworkUnchanged()
    }

    @Test
    fun outsideTapDismissesTheDialogAndKeepsTheNetwork() {
        // the dialog window is outside the compose hierarchy, so tap the scrim as a user would
        val device = UiDevice.getInstance(InstrumentationRegistry.getInstrumentation())
        device.click(device.displayWidth / 2, device.displayHeight / 10)

        assertNetworkUnchanged()
    }

    @Test
    fun systemBackDismissesTheDialogAndKeepsTheNetwork() {
        UiDevice.getInstance(InstrumentationRegistry.getInstrumentation()).pressBack()

        assertNetworkUnchanged()
    }

    private fun assertNetworkUnchanged() {
        compose.waitUntil(timeoutMillis = 5_000) {
            compose.onAllNodes(hasText("Warning: Network Changed")).fetchSemanticsNodes().isEmpty()
        }

        // the settings list is still showing, with the original network the only selected row
        compose.onAllNodesWithText(initialNetwork.toString()).assertCountEquals(1)
        compose.onAllNodesWithContentDescription("Selected").assertCountEquals(1)
        assertEquals(initialNetwork, app.selectedNetwork)
    }
}
