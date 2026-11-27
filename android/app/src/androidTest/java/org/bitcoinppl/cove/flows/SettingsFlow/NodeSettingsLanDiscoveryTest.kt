package org.bitcoinppl.cove.flows.SettingsFlow

import android.system.Os
import androidx.compose.ui.test.hasContentDescription
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.test.platform.app.InstrumentationRegistry
import kotlinx.coroutines.runBlocking
import org.bitcoinppl.cove.AppManager
import org.bitcoinppl.cove.LanNodeDiscovery
import org.bitcoinppl.cove.test.bootstrapRustRuntimeForUiTest
import org.bitcoinppl.cove.test.coveComposeRule
import org.bitcoinppl.cove.ui.theme.CoveTheme
import org.bitcoinppl.cove_core.ApiType
import org.bitcoinppl.cove_core.AppAction
import org.bitcoinppl.cove_core.LanBinding
import org.bitcoinppl.cove_core.LanInterface
import org.bitcoinppl.cove_core.Node
import org.bitcoinppl.cove_core.NodeDiscoveryState
import org.bitcoinppl.cove_core.NodeSelector
import org.bitcoinppl.cove_core.RustNodeDiscoveryManager
import org.bitcoinppl.cove_core.types.Network
import org.json.JSONArray
import org.json.JSONObject
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Assume.assumeTrue
import org.junit.Before
import org.junit.Rule
import org.junit.Test
import java.io.Closeable
import java.net.Inet4Address
import java.net.InetAddress
import java.net.ServerSocket
import java.net.Socket
import kotlin.concurrent.thread

/**
 * Node settings search the local network only when asked, list only the servers for the selected
 * network, and save a picked server through the node check, one choice at a time
 *
 * A UI test can't put an Electrum server on the network the emulator is on, so the app's debug
 * `COVE_UI_TEST_LAN_DISCOVERY` seam, set in process, answers each search with the loopback servers
 * the test scripts. Rust checks those servers as it checks swept ones, so the fake node's Electrum
 * port, which the runner forwards into the device, is listed, and a server on another network
 * isn't. The sweep itself is left to the Rust tests
 */
class NodeSettingsLanDiscoveryTest {
    @get:Rule
    val compose = coveComposeRule()

    private lateinit var app: AppManager
    private lateinit var initialNetwork: Network

    @Before
    fun bootstrap() {
        bootstrapRustRuntimeForUiTest()
        app = AppManager.getInstance()
        initialNetwork = app.selectedNetwork
        changeNetwork(Network.SIGNET)
    }

    @After
    fun restoreNodeAndNetwork() {
        Os.unsetenv(SEARCH_SCRIPT_ENV)

        // the live test moves to mainnet, whose presets don't include the signet default
        changeNetwork(Network.SIGNET)
        NodeSelector().use { it.selectPresetNode(SIGNET_DEFAULT_NODE) }
        changeNetwork(initialNetwork)
    }

    @Test
    fun pickingAServerFoundOnTheNetworkSavesItAsTheSelectedNode() {
        script(Search.Servers(listOf(FAKE_SERVER_HOST to FAKE_NODE_ELECTRUM_PORT)))
        showScreen()

        // a search, even one made on its own, would turn the button into Search Again
        compose.waitForIdle()
        compose.onNodeWithText("Find Nodes on This Network").assertExists()
        assertTrue("opening node settings must not search the network", !shown(FAKE_SERVER_HOST))

        compose.onNodeWithText("Find Nodes on This Network").performClick()
        compose.waitUntil(timeoutMillis = 10_000) { shown(FAKE_SERVER_HOST) }

        compose.onNodeWithText(FAKE_SERVER_HOST).performClick()
        compose.waitUntil(timeoutMillis = 30_000) { selectedNode().url == FAKE_NODE_URL }

        assertEquals(ApiType.ELECTRUM, selectedNode().apiType)
        compose.waitUntil(timeoutMillis = 5_000) { shownAsSelected(FAKE_SERVER_HOST) }
    }

    // every server is asked which chain it serves, so one on another network is never offered
    @Test
    fun aServerOnAnotherNetworkIsNotListed() {
        ElectrumTestServer(MAINNET_GENESIS).use { mainnet ->
            script(Search.Servers(listOf(MAINNET_SERVER_HOST to mainnet.port, FAKE_SERVER_HOST to FAKE_NODE_ELECTRUM_PORT)))
            showScreen()

            compose.onNodeWithText("Find Nodes on This Network").performClick()
            compose.waitUntil(timeoutMillis = 15_000) { shown("Search Again") }

            assertTrue("the signet server wasn't listed", shown(FAKE_SERVER_HOST))
            assertTrue("a mainnet server was listed on signet", !shown(MAINNET_SERVER_HOST))
            assertTrue("the mainnet server was never asked", mainnet.requests > 0)
        }
    }

    // stopping a search keeps the servers it already found
    @Test
    fun stopKeepsTheServersFoundSoFar() {
        script(Search.Servers(listOf(FAKE_SERVER_HOST to FAKE_NODE_ELECTRUM_PORT), keepSearching = true))
        showScreen()

        compose.onNodeWithText("Find Nodes on This Network").performClick()
        compose.waitUntil(timeoutMillis = 10_000) { shown(FAKE_SERVER_HOST) }
        assertTrue("a running search isn't shown as searching", shown("Searching this network…"))

        compose.onNodeWithText("Stop").performClick()
        compose.waitUntil(timeoutMillis = 5_000) { shown("Search Again") }

        assertTrue("stopping dropped the server found", shown(FAKE_SERVER_HOST))
        assertTrue("a stopped search is still shown as searching", !shown("Searching this network…"))
        assertTrue("a stopped search says it found nothing", !shown("No Electrum servers found on this network"))
    }

    @Test
    fun aSearchThatFindsNothingSaysSoAndSearchAgainSearchesAgain() {
        script(Search.Servers(emptyList()), Search.Servers(listOf(FAKE_SERVER_HOST to FAKE_NODE_ELECTRUM_PORT)))
        showScreen()

        compose.onNodeWithText("Find Nodes on This Network").performClick()
        compose.waitUntil(timeoutMillis = 10_000) { shown("No Electrum servers found on this network") }

        compose.onNodeWithText("Search Again").performClick()
        compose.waitUntil(timeoutMillis = 10_000) { shown(FAKE_SERVER_HOST) }
        assertTrue("the empty search's message stayed", !shown("No Electrum servers found on this network"))
    }

    // one selection is checked at a time, so a slow check can't finish after a later pick and
    // replace it
    @Test
    fun aPresetTappedWhileAFoundServerIsCheckedIsIgnoredUntilTheCheckEnds() {
        val saved = Node(name = FAKE_SERVER_HOST, network = Network.SIGNET, apiType = ApiType.ELECTRUM, url = FAKE_NODE_URL)
        runBlocking { NodeSelector().use { it.checkAndSaveNode(saved) } }

        // a server that proves it serves signet but never answers the node check holds that
        // check open until it times out
        ElectrumTestServer(SIGNET_GENESIS).use { slow ->
            script(Search.Servers(listOf(SLOW_SERVER_HOST to slow.port)))
            showScreen()

            compose.onNodeWithText("Find Nodes on This Network").performClick()
            compose.waitUntil(timeoutMillis = 10_000) { shown(SLOW_SERVER_HOST) }
            compose.onNodeWithText(SLOW_SERVER_HOST).performClick()

            compose.onNodeWithText(SIGNET_DEFAULT_NODE).performClick()
            compose.waitForIdle()
            assertEquals("a preset can't replace a node still being checked", saved, selectedNode())

            compose.waitUntil(timeoutMillis = 40_000) { shown("Connection Failed") }
            compose.onNodeWithText("OK").performClick()
        }

        compose.onNodeWithText(SIGNET_DEFAULT_NODE).performClick()
        compose.waitUntil(timeoutMillis = 10_000) { selectedNode().name == SIGNET_DEFAULT_NODE }
    }

    @Test
    fun searchingOffWifiAsksToConnectToWifi() {
        script(Search.Fails("noLocalNetwork"))
        showScreen()

        compose.onNodeWithText("Find Nodes on This Network").performClick()
        compose.waitUntil(timeoutMillis = 10_000) { shown("Connect to Wi-Fi to find nodes on your network") }
        compose.onNodeWithText("Search Again").assertExists()
    }

    @Test
    fun searchingBehindABlockingVpnSaysTheVpnBlocksIt() {
        script(Search.Fails("localNetworkBlocked"))
        showScreen()

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
            RustNodeDiscoveryManager().use { manager ->
                manager.start(listOf(lan))
                compose.waitUntil(timeoutMillis = 40_000) { manager.state() !is NodeDiscoveryState.Searching }

                (manager.state() as? NodeDiscoveryState.Finished)?.found.orEmpty()
            }.firstOrNull { it.node.url == expectedUrl }

        assertTrue("$expectedUrl was not found", found != null)
        runBlocking { NodeSelector().use { it.checkSelectedNode(found!!.node) } }
    }

    private sealed interface Search {
        data class Fails(
            val error: String,
        ) : Search

        data class Servers(
            val servers: List<Pair<String, Int>>,
            val keepSearching: Boolean = false,
        ) : Search
    }

    /** Sets what the app's seam answers each search with, in order, the last for every later one */
    private fun script(vararg searches: Search) {
        val entries =
            searches.map { search ->
                when (search) {
                    is Search.Fails -> JSONObject().put("error", search.error)
                    is Search.Servers ->
                        JSONObject()
                            .put("servers", JSONArray(search.servers.map { (host, port) -> JSONObject().put("host", host).put("port", port) }))
                            .put("keepSearching", search.keepSearching)
                }
            }

        Os.setenv(SEARCH_SCRIPT_ENV, JSONObject().put("searches", JSONArray(entries)).toString(), true)
    }

    private fun showScreen() {
        compose.setContent {
            CoveTheme {
                NodeSettingsScreen(app = app)
            }
        }
    }


    private fun shown(
        text: String,
        substring: Boolean = false,
    ) = compose.onAllNodes(hasText(text, substring = substring)).fetchSemanticsNodes().isNotEmpty()

    // a found server's row merges its texts and its check mark, whose description is Selected
    private fun shownAsSelected(host: String) =
        compose
            .onAllNodes(hasText(host) and hasContentDescription("Selected"))
            .fetchSemanticsNodes()
            .isNotEmpty()

    private fun selectedNode() = NodeSelector().use { selector -> selector.selectedNode().toNode() }

    private fun changeNetwork(network: Network) {
        compose.runOnUiThread { app.dispatch(AppAction.ChangeNetwork(network)) }
        compose.waitUntil(timeoutMillis = 5_000) { app.selectedNetwork == network }
    }

    private companion object {
        const val SEARCH_SCRIPT_ENV = "COVE_UI_TEST_LAN_DISCOVERY"
        const val SIGNET_DEFAULT_NODE = "mutinynet"
        const val FAKE_SERVER_HOST = "fake-electrs.local"
        const val MAINNET_SERVER_HOST = "mainnet-electrs.local"
        const val SLOW_SERVER_HOST = "slow-electrs.local"

        // the fake node's Electrum port, which the runner forwards into the device
        const val FAKE_NODE_ELECTRUM_PORT = 39_002
        const val FAKE_NODE_URL = "tcp://127.0.0.1:39002"

        const val SIGNET_GENESIS = "00000008819873e925422c1ff0f99f7cc9bbb232af63a077a480a3633bee1ef6"
        const val MAINNET_GENESIS = "000000000019d6689c085ae165831e934ff763ae46a2a6c172b3f1b60a8ce26f"
    }
}

/**
 * An Electrum server on this device's IPv4 loopback that proves it serves the chain whose genesis
 * block is `genesisHash` and never answers anything else, so a node check of it runs until it
 * times out
 *
 * It listens on 127.0.0.1, since Android's getLoopbackAddress is the IPv6 one
 */
private class ElectrumTestServer(
    private val genesisHash: String,
) : Closeable {
    private val socket = ServerSocket(0, 8, InetAddress.getByName("127.0.0.1"))
    val port: Int = socket.localPort

    @Volatile
    var requests = 0
        private set

    init {
        thread(isDaemon = true, name = "electrum-test-server") {
            while (!socket.isClosed) {
                val connection = runCatching { socket.accept() }.getOrNull() ?: break
                thread(isDaemon = true) { answer(connection) }
            }
        }
    }

    private fun answer(connection: Socket) {
        connection.use {
            val reader = it.getInputStream().bufferedReader()
            val writer = it.getOutputStream().bufferedWriter()

            while (true) {
                val request = JSONObject(reader.readLine() ?: return)
                requests += 1

                val result: Any =
                    when (request.getString("method")) {
                        "server.version" -> JSONArray(listOf("Electrum test server 1.0", "1.4"))
                        "server.features" -> JSONObject().put("genesis_hash", genesisHash).put("server_version", "Electrum test server 1.0")
                        else -> continue
                    }

                writer.write(JSONObject().put("jsonrpc", "2.0").put("id", request.get("id")).put("result", result).toString())
                writer.newLine()
                writer.flush()
            }
        }
    }

    override fun close() {
        socket.close()
    }
}
