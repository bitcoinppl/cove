package org.bitcoinppl.cove.flows.keyteleport

import androidx.activity.ComponentActivity
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
import org.bitcoinppl.cove_core.KeyTeleportManagerAction
import org.bitcoinppl.cove_core.KeyTeleportManagerState
import org.bitcoinppl.cove_core.KeyTeleportRoute
import org.bitcoinppl.cove_core.Mnemonic
import org.bitcoinppl.cove_core.Route
import org.bitcoinppl.cove_core.types.WalletId
import org.junit.After
import org.junit.Assert.assertEquals
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
 * each review, so Import Wallet has to create it rather than find the existing one
 */
class KeyTeleportMnemonicReviewTest {
    @get:Rule
    val compose = createAndroidComposeRule<ComponentActivity>()

    // the wallet this test created last, deleted afterwards
    private var walletId: WalletId? = null

    @Before
    fun setUp() {
        bootstrapRustRuntimeForUiTest()

        // receiving deletes the stored receive session afterwards, so never run over someone's pending transfer
        assumeTrue("a KeyTeleport receive session is already stored on this device", !hasStoredReceiveSession())

        // the test deletes wallets for its words, so never run it over one it did not create
        assumeTrue("a wallet for the test words already exists on this device", walletsForTheTestWords() == 0)
    }

    @After
    fun tearDown() {
        val app = AppManager.getInstance()
        try {
            // setUp found no stored session, so one here is this test's own, left by a failure
            if (hasStoredReceiveSession()) endReceive(app.getKeyTeleportManager())
        } finally {
            InstrumentationRegistry.getInstrumentation().runOnMainSync { app.clearKeyTeleportManager() }
            walletId?.let(::deleteTestWallet)
        }
    }

    @Test
    fun finishWithoutImportingAddsNoWalletAndImportWalletAddsOne() {
        val app = AppManager.getInstance()
        compose.setContent {
            CoveTheme(dynamicColor = false) {
                KeyTeleportContainer(app = app, route = KeyTeleportRoute.RECEIVE)
            }
        }

        val receiver = app.getKeyTeleportManager()

        // the words start hidden, reveal on tap, and finishing leaves no wallet behind
        reachReview(receiver)
        assertNoWordsInTree("before revealing")
        compose.onNodeWithContentDescription(RECOVERY_WORDS_HINT).performClick()
        compose.waitUntil(STATE_TIMEOUT_MILLIS) { wordsInTree() == TEST_WORDS.toSet().size }
        compose.onNodeWithText("Finish Without Importing").performClick()
        compose.waitUntil(STATE_TIMEOUT_MILLIS) { receiver.state !is KeyTeleportManagerState.ReceiveMnemonicReview }
        assertEquals("Finish Without Importing added a wallet", 0, walletsForTheTestWords())

        // a fresh request, then Import Wallet adds the wallet once and Done opens it
        receiver.dispatch(KeyTeleportManagerAction.StartReceive)
        reachReview(receiver)
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
     * Import the test words, send them to [receiver] from a second manager, delete them again, and
     * enter the password on screen to reach the review
     */
    private fun reachReview(receiver: KeyTeleportManager) {
        val request = awaitRequest(receiver)
        val sender = importTestWalletOrNull(TEST_WALLET)
        assertTrue("the test words were already on this device", sender != null)
        val senderId = requireNotNull(sender).id
        walletId = senderId

        val response = KeyTeleportTestPeer().use { peer -> peer.sendWallet(senderId, request) }
        deleteTestWallet(senderId)
        walletId = null
        awaitOnMain("the sending wallet to be deleted") { (walletsForTheTestWords() == 0).takeIf { it } }

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

    private fun readsAloud(value: String): SemanticsMatcher =
        SemanticsMatcher("reads a secret value aloud") { node ->
            val config = node.config
            val texts =
                config.getOrNull(SemanticsProperties.Text).orEmpty().map { it.text } +
                    config.getOrNull(SemanticsProperties.ContentDescription).orEmpty()

            texts.any { it == value }
        }

    private fun walletsForTheTestWords(): Int =
        Database().wallets().all().count { it.masterFingerprint?.asLowercase() == TEST_WALLET.masterFingerprint }

    private companion object {
        const val STATE_TIMEOUT_MILLIS = 15_000L
        const val RECOVERY_WORDS_HINT = "Tap to reveal recovery words"

        // published BIP39 test vector, never used for funds; distinct from the vectors other tests import
        val TEST_WORDS =
            listOf(
                "hamster", "diagram", "private", "dutch", "cause", "delay",
                "private", "meat", "slide", "toddler", "razor", "book",
            )
        val TEST_WALLET = TestWalletWords(TEST_WORDS, masterFingerprint = "ee25b464")
    }
}
