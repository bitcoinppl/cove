package org.bitcoinppl.cove.flows.keyteleport

import androidx.activity.ComponentActivity
import androidx.compose.runtime.getValue
import androidx.compose.runtime.key
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.semantics.getOrNull
import androidx.compose.ui.test.SemanticsMatcher
import androidx.compose.ui.test.hasSetTextAction
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performTextInput
import androidx.test.platform.app.InstrumentationRegistry
import org.bitcoinppl.cove.AppManager
import org.bitcoinppl.cove.test.TestWalletWords
import org.bitcoinppl.cove.test.awaitOnMain
import org.bitcoinppl.cove.test.bootstrapRustRuntimeForUiTest
import org.bitcoinppl.cove.test.deleteTestWallet
import org.bitcoinppl.cove.test.importTestWalletOrNull
import org.bitcoinppl.cove.ui.theme.CoveTheme
import org.bitcoinppl.cove_core.Database
import org.bitcoinppl.cove_core.KeyTeleportManagerState
import org.bitcoinppl.cove_core.KeyTeleportRoute
import org.bitcoinppl.cove_core.Mnemonic
import org.bitcoinppl.cove_core.Route
import org.bitcoinppl.cove_core.RouteFactory
import org.bitcoinppl.cove_core.types.WalletId
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Assume.assumeTrue
import org.junit.Before
import org.junit.Rule
import org.junit.Test

/**
 * The received recovery words review keeps the words hidden until revealed, Finish Without
 * Importing adds no wallet, and Import Wallet adds exactly one and opens it
 *
 * Hosts the real KeyTeleport receive screen. A second KeyTeleport manager plays the sending phone,
 * in place of the camera between two devices. The sending wallet is deleted from this phone before
 * each review, so Import Wallet has to create it rather than find the existing one. Deleting it
 * changes the app's routes, which closes a KeyTeleport manager no route shows, so the screen is
 * reopened from the stored receive session the way a relaunch during a receive would
 */
class KeyTeleportMnemonicReviewTest {
    @get:Rule
    val compose = createAndroidComposeRule<ComponentActivity>()

    // the wallet this test created last, deleted afterwards
    private var walletId: WalletId? = null

    // set once setUp's guards pass, so a skipped test never ends a session it found stored
    private var ownsReceiveSession = false

    // bumped to recompose the hosted screen with a fresh manager from AppManager
    private var hostGeneration by mutableIntStateOf(0)

    @Before
    fun setUp() {
        bootstrapRustRuntimeForUiTest()

        // receiving deletes the stored receive session afterwards, so never run over someone's pending transfer
        assumeTrue("a KeyTeleport receive session is already stored on this device", !hasStoredReceiveSession())

        // the test deletes wallets for its words, so never run it over one it did not create
        assumeTrue("a wallet for the test words already exists on this device", walletsForTheTestWords() == 0)
        ownsReceiveSession = true
    }

    @After
    fun tearDown() {
        val app = AppManager.getInstance()
        try {
            // setUp found no stored session, so one here is this test's own, left by a failure
            if (ownsReceiveSession && hasStoredReceiveSession()) endReceive(app.getKeyTeleportManager())
        } finally {
            InstrumentationRegistry.getInstrumentation().runOnMainSync { app.clearKeyTeleportManager() }
            walletId?.let(::deleteTestWallet)
        }
    }

    @Test
    fun finishWithoutImportingAddsNoWalletAndImportWalletAddsOne() {
        val app = AppManager.getInstance()
        compose.setContent {
            key(hostGeneration) {
                CoveTheme(dynamicColor = false) {
                    KeyTeleportContainer(app = app, route = KeyTeleportRoute.RECEIVE)
                }
            }
        }

        // the words start hidden, reveal on tap at their own positions, and finishing leaves no wallet
        var receiver = reachReview()
        assertNoWordsInTree("before revealing")
        compose.onNodeWithContentDescription(RECOVERY_WORDS_HINT).performClick()
        compose.waitUntil(STATE_TIMEOUT_MILLIS) { wordsInTree() > 0 }
        assertEachWordAtItsPosition()

        // the screen sits outside the router here, so give Finish a route of its own to close
        onMain { app.pushRoute(RECEIVE_ROUTE) }
        compose.onNodeWithText("Finish Without Importing").performClick()
        compose.waitUntil(STATE_TIMEOUT_MILLIS) { receiver.state !is KeyTeleportManagerState.ReceiveMnemonicReview }
        compose.waitForIdle()
        assertFalse("Finish Without Importing left the receive screen open", onMainValue { RECEIVE_ROUTE in app.router.routes })
        assertEquals("Finish Without Importing added a wallet", 0, walletsForTheTestWords())

        // Finish closed the screen, so open it again for a fresh request, then Import Wallet adds
        // the wallet once and Done opens it
        reopenReceiveScreen()
        receiver = reachReview()
        assertNoWordsInTree("on the second review")
        compose.onNodeWithText("Import Wallet").performClick()
        val imported =
            awaitOnMain("the received words to import") {
                (receiver.state as? KeyTeleportManagerState.ReceiveImportedWallet)?.v1?.id
            }

        walletId = imported
        compose.onNodeWithText("Wallet imported").assertExists()
        compose.onNodeWithText("Done").performClick()
        awaitOnMain("Done to open the imported wallet") {
            (app.router.default == Route.SelectedWallet(imported)).takeIf { it }
        }

        assertEquals("Import Wallet did not add exactly one wallet", 1, walletsForTheTestWords())
        assertEquals("the imported wallet holds different words", TEST_WORDS, Mnemonic(id = imported).use { it.words() })
    }

    /**
     * Import the test words, send them from a second manager to the receive screen's request,
     * delete them again, and enter the password on screen to reach the review
     *
     * Returns the screen's manager, which is a new one after the reopen
     */
    private fun reachReview(): KeyTeleportManager {
        val request = awaitRequest(currentReceiver())
        val sender = importTestWalletOrNull(TEST_WALLET)
        assertTrue("the test words were already on this device", sender != null)
        val senderId = requireNotNull(sender).id
        walletId = senderId

        val response = KeyTeleportTestPeer().use { peer -> peer.sendWallet(senderId, request) }
        deleteTestWallet(senderId)
        walletId = null
        awaitOnMain("the sending wallet to be deleted") { (walletsForTheTestWords() == 0).takeIf { it } }

        val receiver = reopenReceiveScreen()
        assertEquals("the reopened screen did not resume the same request", request, awaitRequest(receiver))
        val accepted =
            awaitOnMain("the receiver to read the sender response") {
                receiver.ingestKeyTeleportText(response.packetText, KeyTeleportFlowDirection.RECEIVE)
            }

        assertTrue("the receiver rejected the sender response", accepted)
        compose.waitUntil(STATE_TIMEOUT_MILLIS) { receiver.state is KeyTeleportManagerState.ReceiveEnterPassword }
        compose.onNode(hasSetTextAction()).performTextInput(response.password)
        compose.onNodeWithText("Continue").performClick()
        compose.waitUntil(STATE_TIMEOUT_MILLIS) { receiver.state is KeyTeleportManagerState.ReceiveMnemonicReview }
        compose.waitForIdle()

        return receiver
    }

    private fun currentReceiver(): KeyTeleportManager = onMainValue { AppManager.getInstance().getKeyTeleportManager() }

    // closing keeps the stored receive session, which the new screen's manager resumes
    private fun reopenReceiveScreen(): KeyTeleportManager {
        onMain {
            AppManager.getInstance().clearKeyTeleportManager()
            hostGeneration += 1
        }

        compose.waitForIdle()
        return currentReceiver()
    }

    // each chip merges its position label and word, so a word under the wrong position fails here
    private fun assertEachWordAtItsPosition() {
        val misplaced =
            TEST_WORDS.withIndex().count { (index, word) ->
                compose
                    .onAllNodes(showsWordAt(index + 1, word), useUnmergedTree = false)
                    .fetchSemanticsNodes()
                    .isEmpty()
            }

        assertEquals("$misplaced recovery word(s) were missing from their positions", 0, misplaced)
    }

    // words never go into assertion messages, which end up in test reports
    private fun assertNoWordsInTree(moment: String) {
        val leaked = wordsInTree()
        assertTrue("$leaked recovery word(s) were in the UI tree $moment", leaked == 0)
    }

    /**
     * How many distinct test words the merged semantics tree reads aloud, matching each word whole
     * since each sits in its own text node
     */
    private fun wordsInTree(): Int =
        TEST_WORDS.toSet().count { word ->
            compose
                .onAllNodes(readsAloud(word), useUnmergedTree = false)
                .fetchSemanticsNodes()
                .isNotEmpty()
        }

    private fun showsWordAt(
        position: Int,
        word: String,
    ): SemanticsMatcher =
        SemanticsMatcher("shows a secret word at its position") { node ->
            val texts = node.config.getOrNull(SemanticsProperties.Text).orEmpty().map { it.text }
            texts == listOf("$position.", word)
        }

    private fun readsAloud(value: String): SemanticsMatcher =
        SemanticsMatcher("reads a secret value aloud") { node ->
            val config = node.config
            val texts =
                config.getOrNull(SemanticsProperties.Text).orEmpty().map { it.text } +
                    config.getOrNull(SemanticsProperties.ContentDescription).orEmpty()

            texts.any { it == value }
        }

    private fun onMain(block: () -> Unit) {
        InstrumentationRegistry.getInstrumentation().runOnMainSync(block)
    }

    private fun <T : Any> onMainValue(read: () -> T): T = awaitOnMain("a value from the main thread", read = read)

    private fun walletsForTheTestWords(): Int =
        Database().wallets().all().count { it.masterFingerprint?.asLowercase() == TEST_WALLET.masterFingerprint }

    private companion object {
        const val STATE_TIMEOUT_MILLIS = 15_000L
        const val RECOVERY_WORDS_HINT = "Tap to reveal recovery words"
        val RECEIVE_ROUTE = RouteFactory().keyTeleportReceive()

        // published BIP39 test vector, never used for funds; distinct from the vectors other tests import
        val TEST_WORDS =
            listOf(
                "hamster", "diagram", "private", "dutch", "cause", "delay",
                "private", "meat", "slide", "toddler", "razor", "book",
            )
        val TEST_WALLET = TestWalletWords(TEST_WORDS, masterFingerprint = "ee25b464")
    }
}
