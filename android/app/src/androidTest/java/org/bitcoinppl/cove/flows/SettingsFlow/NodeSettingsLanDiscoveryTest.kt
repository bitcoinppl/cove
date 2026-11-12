package org.bitcoinppl.cove.flows.SettingsFlow

import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.test.platform.app.InstrumentationRegistry
import kotlinx.coroutines.runBlocking
import org.bitcoinppl.cove.AppManager
import org.bitcoinppl.cove.LanNodeDiscovery
import org.bitcoinppl.cove.test.bootstrapRustRuntimeForUiTest
import org.bitcoinppl.cove.ui.theme.CoveTheme
import org.bitcoinppl.cove_core.ApiType
import org.bitcoinppl.cove_core.AppAction
import org.bitcoinppl.cove_core.DiscoveredNode
import org.bitcoinppl.cove_core.LanBinding
import org.bitcoinppl.cove_core.LanInterface
import org.bitcoinppl.cove_core.Node
import org.bitcoinppl.cove_core.NodeDiscoveryException
import org.bitcoinppl.cove_core.NodeSelector
import org.bitcoinppl.cove_core.types.Network
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Assume.assumeTrue
import org.junit.Before
import org.junit.Rule
import org.junit.Test
import java.net.Inet4Address
import java.net.InetAddress
import java.net.ServerSocket

/**
 * Node settings search the local network only when asked, and picking a server found there
 * checks it and saves it as the selected node
 *
 * The search itself is Rust's and its tests cover it; these tests feed the screen a server found
 * at the fake node's Electrum port, which the runner forwards into the emulator
 */
class NodeSettingsLanDiscoveryTest {
    @get:Rule
    val compose = createComposeRule()

    private lateinit var app: AppManager
    private lateinit var initialNetwork: Network
    private var searches = 0

    @Before
    fun bootstrap() {
        bootstrapRustRuntimeForUiTest()
        app = AppManager.getInstance()
        initialNetwork = app.selectedNetwork
        changeNetwork(Network.SIGNET)
    }

    @After
    fun restoreNodeAndNetwork() {
        // the live test moves to mainnet, whose presets don't include the signet default
        changeNetwork(Network.SIGNET)
        NodeSelector().use { it.selectPresetNode(SIGNET_DEFAULT_NODE) }
        changeNetwork(initialNetwork)
    }

    @Test
    fun pickingAServerFoundOnTheNetworkSavesItAsTheSelectedNode() {
        val found = discoveredFakeNode()
        showScreen { listOf(found) }

        compose.waitForIdle()
        assertEquals("opening node settings must not search the network", 0, searches)

        compose.onNodeWithText("Find Nodes on This Network").performClick()
        compose.waitUntil(timeoutMillis = 10_000) { shown(found.host) }
        assertEquals(1, searches)

        compose.onNodeWithText(found.host).performClick()
        compose.waitUntil(timeoutMillis = 30_000) { selectedNode().url == found.node.url }

        assertEquals(found.node, selectedNode())
    }

    // one selection is checked at a time, so a slow check can't finish after a later pick and
    // replace it
    @Test
    fun aPresetTappedWhileAFoundServerIsCheckedIsIgnoredUntilTheCheckEnds() {
        val saved = discoveredFakeNode()
        runBlocking { NodeSelector().use { it.checkAndSaveNode(saved.node) } }

        // a server that accepts but never answers holds the check open until it times out; it
        // listens on IPv4 loopback, since Android's getLoopbackAddress is the IPv6 one
        ServerSocket(0, 1, InetAddress.getByName("127.0.0.1")).use { silent ->
            val slow = discoveredNode(host = "silent-electrs.local", port = silent.localPort)
            showScreen { listOf(slow) }

            compose.onNodeWithText("Find Nodes on This Network").performClick()
            compose.waitUntil(timeoutMillis = 10_000) { shown(slow.host) }
            compose.onNodeWithText(slow.host).performClick()

            compose.onNodeWithText(SIGNET_DEFAULT_NODE).performClick()
            compose.waitForIdle()
            assertEquals("a preset can't replace a node still being checked", saved.node, selectedNode())

            compose.waitUntil(timeoutMillis = 40_000) { shown("Connection Failed") }
            compose.onNodeWithText("OK").performClick()
        }

        compose.onNodeWithText(SIGNET_DEFAULT_NODE).performClick()
        compose.waitUntil(timeoutMillis = 10_000) { selectedNode().name == SIGNET_DEFAULT_NODE }
    }

    @Test
    fun searchingOffWifiAsksToConnectToWifi() {
        showScreen { throw NodeDiscoveryException.NoLocalNetwork() }

        compose.onNodeWithText("Find Nodes on This Network").performClick()
        compose.waitUntil(timeoutMillis = 10_000) { shown("Connect to Wi-Fi to find nodes on your network") }
        compose.onNodeWithText("Search Again").assertExists()
    }

    @Test
    fun searchingBehindABlockingVpnSaysTheVpnBlocksIt() {
        showScreen { throw NodeDiscoveryException.LocalNetworkBlocked() }

        compose.onNodeWithText("Find Nodes on This Network").performClick()
        compose.waitUntil(timeoutMillis = 10_000) { shown("A VPN is blocking this network", substring = true) }
    }

    // the interfaces discovery sweeps must be the emulator's own Wi-Fi or Ethernet network, each
    // carrying the handle of the network its sockets are bound to
    @Test
    fun lanInterfacesAreTheDevicesPrivateIpv4Networks() {
        val context = InstrumentationRegistry.getInstrumentation().targetContext
        val interfaces = LanNodeDiscovery(context).lanInterfaces()

        assertTrue("the emulator is on a Wi-Fi or Ethernet network", interfaces.isNotEmpty())
        for (lan in interfaces) {
            val address = InetAddress.getByName(lan.ipv4Address)
            assertTrue("$lan is IPv4", address is Inet4Address)
            assertTrue("$lan is a private address", address.isSiteLocalAddress || address.isLinkLocalAddress)
            assertTrue("$lan is bound to an Android network", lan.binding is LanBinding.AndroidNetwork)
        }
    }

    /**
     * Runs the real search on the local network this machine is on and connects to what it finds
     *
     * Skipped unless the runner passes `-e coveLiveLanInterface 192.168.1.18/24 -e coveLiveLanNode
     * tcp://192.168.1.10:50001`, an interface address on the host's network and the node expected
     * there. The emulator's traffic leaves through the host, so the search is bound to the
     * emulator's own network, as a real search is, while sweeping the host's subnet
     */
    @Test
    fun liveSearchFindsTheNodeOnTheHostsNetwork() {
        val arguments = InstrumentationRegistry.getArguments()
        val interfaceArgument = arguments.getString("coveLiveLanInterface")
        val expectedUrl = arguments.getString("coveLiveLanNode")
        assumeTrue("live local network test not requested", interfaceArgument != null && expectedUrl != null)

        changeNetwork(Network.BITCOIN)
        val (address, prefix) = interfaceArgument!!.split("/", limit = 2)
        val context = InstrumentationRegistry.getInstrumentation().targetContext
        val emulatorNetwork = LanNodeDiscovery(context).lanInterfaces().first().binding
        val lan = LanInterface(ipv4Address = address, prefixLength = prefix.toUByte(), binding = emulatorNetwork)

        val found =
            runBlocking { NodeSelector().use { it.discoverLanNodes(listOf(lan)) } }
                .firstOrNull { it.node.url == expectedUrl }

        assertTrue("$expectedUrl was not found", found != null)
        runBlocking { NodeSelector().use { it.checkSelectedNode(found!!.node) } }
    }

    private fun showScreen(search: suspend () -> List<DiscoveredNode>) {
        compose.setContent {
            CoveTheme {
                NodeSettingsScreen(
                    app = app,
                    discoverLanNodes = {
                        searches += 1
                        search()
                    },
                )
            }
        }
    }

    private fun discoveredFakeNode() = discoveredNode(host = "fake-electrs.local", port = FAKE_NODE_ELECTRUM_PORT)

    private fun discoveredNode(
        host: String,
        port: Int,
    ): DiscoveredNode {
        val address = "127.0.0.1:$port"
        val node = Node(name = host, network = Network.SIGNET, apiType = ApiType.ELECTRUM, url = "tcp://$address")

        return DiscoveredNode(node = node, host = host, address = address, software = null)
    }

    private fun shown(
        text: String,
        substring: Boolean = false,
    ) = compose.onAllNodes(hasText(text, substring = substring)).fetchSemanticsNodes().isNotEmpty()

    private fun selectedNode() = NodeSelector().use { selector -> selector.selectedNode().toNode() }

    private fun changeNetwork(network: Network) {
        compose.runOnUiThread { app.dispatch(AppAction.ChangeNetwork(network)) }
        compose.waitUntil(timeoutMillis = 5_000) { app.selectedNetwork == network }
    }

    private companion object {
        const val SIGNET_DEFAULT_NODE = "mutinynet"

        // the fake node's Electrum port, which the runner forwards into the device
        const val FAKE_NODE_ELECTRUM_PORT = 39_002
    }
}
