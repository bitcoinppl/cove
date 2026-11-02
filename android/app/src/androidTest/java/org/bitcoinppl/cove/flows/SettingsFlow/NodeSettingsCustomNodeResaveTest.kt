package org.bitcoinppl.cove.flows.SettingsFlow

import androidx.compose.ui.test.assert
import androidx.compose.ui.test.hasSetTextAction
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.isEnabled
import androidx.compose.ui.test.isNotEnabled
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performTextReplacement
import okhttp3.mockwebserver.Dispatcher
import okhttp3.mockwebserver.MockResponse
import okhttp3.mockwebserver.MockWebServer
import okhttp3.mockwebserver.RecordedRequest
import org.bitcoinppl.cove.AppManager
import org.bitcoinppl.cove.test.bootstrapRustRuntimeForUiTest
import org.bitcoinppl.cove.ui.theme.CoveTheme
import org.bitcoinppl.cove_core.ApiType
import org.bitcoinppl.cove_core.AppAction
import org.bitcoinppl.cove_core.NodeSelector
import org.bitcoinppl.cove_core.types.Network
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Before
import org.junit.Rule
import org.junit.Test
import java.net.InetAddress

/**
 * Saving the selected custom node a second time keeps it
 *
 * After the first save the screen selects the custom node by its own name, and the second save
 * used to pass that name to Rust as the node type, so a node named "Home node" was parsed as
 * neither Electrum nor Esplora and replaced with the network's default node
 */
class NodeSettingsCustomNodeResaveTest {
    @get:Rule
    val compose = createComposeRule()

    private val esplora = MockWebServer()
    private lateinit var app: AppManager
    private lateinit var initialNetwork: Network

    @Before
    fun hostScreenOnSignetWithALocalEsplora() {
        // the node check only asks for the tip height, so any number is a reachable Esplora
        esplora.dispatcher =
            object : Dispatcher() {
                override fun dispatch(request: RecordedRequest) = MockResponse().setBody("100")
            }

        // a literal ip, because the node url rules reject a host name without a dot
        esplora.start(InetAddress.getByName(LOOPBACK), 0)

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
    fun restoreNodeAndNetwork() {
        NodeSelector().use { it.selectPresetNode(SIGNET_DEFAULT_NODE) }
        changeNetwork(initialNetwork)
        esplora.shutdown()
    }

    @Test
    fun savingACustomEsploraNodeAgainKeepsIt() {
        val url = "http://$LOOPBACK:${esplora.port}/api"

        compose.onNodeWithText("Custom Esplora").performClick()
        compose.onNode(hasSetTextAction() and hasText("URL")).performTextReplacement(url)
        compose.onNode(hasSetTextAction() and hasText("Name")).performTextReplacement(NODE_NAME)
        compose.onNodeWithText("Save Custom Node").performClick()

        compose.waitUntil(timeoutMillis = 30_000) { selectedNode().url == url }
        val saved = selectedNode()
        assertEquals(ApiType.ESPLORA, saved.apiType)
        assertEquals(NODE_NAME, saved.name)

        compose.waitUntil(timeoutMillis = 10_000) { saveButtonIsEnabled() }
        val requestsBeforeResave = esplora.requestCount
        compose.onNodeWithText("Save Custom Node").performClick()

        // done once the screen stops saving after checking our node, failing, or saving another
        // node, which hides the custom form and its save button
        compose.waitUntil(timeoutMillis = 30_000) {
            val settled =
                esplora.requestCount > requestsBeforeResave ||
                    compose.onAllNodes(hasText("Connection Failed")).fetchSemanticsNodes().isNotEmpty() ||
                    selectedNode() != saved

            settled && !isSaving()
        }

        assertEquals("the saved custom node must survive a second save", saved, selectedNode())
        compose.onNode(hasSetTextAction() and hasText("URL")).assert(hasText(url))
    }

    private fun selectedNode() = NodeSelector().use { selector -> selector.selectedNode().toNode() }

    private fun saveButtonIsEnabled() =
        compose.onAllNodes(hasText("Save Custom Node") and isEnabled()).fetchSemanticsNodes().isNotEmpty()

    // the save button is disabled only while a save is in flight
    private fun isSaving() =
        compose.onAllNodes(hasText("Save Custom Node") and isNotEnabled()).fetchSemanticsNodes().isNotEmpty()

    private fun changeNetwork(network: Network) {
        compose.runOnUiThread { app.dispatch(AppAction.ChangeNetwork(network)) }
        compose.waitUntil(timeoutMillis = 5_000) { app.selectedNetwork == network }
    }

    private companion object {
        const val LOOPBACK = "127.0.0.1"
        const val NODE_NAME = "Home node"
        const val SIGNET_DEFAULT_NODE = "mutinynet"
    }
}
