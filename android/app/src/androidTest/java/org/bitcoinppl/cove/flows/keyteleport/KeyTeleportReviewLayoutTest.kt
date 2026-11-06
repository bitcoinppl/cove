package org.bitcoinppl.cove.flows.keyteleport

import androidx.activity.ComponentActivity
import androidx.compose.ui.geometry.Rect
import androidx.compose.ui.semantics.SemanticsActions
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.semantics.getOrNull
import androidx.compose.ui.test.SemanticsMatcher
import androidx.compose.ui.test.SemanticsNodeInteraction
import androidx.compose.ui.test.assertIsEnabled
import androidx.compose.ui.test.hasClickAction
import androidx.compose.ui.test.hasContentDescription
import androidx.compose.ui.test.hasSetTextAction
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.onRoot
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performScrollTo
import androidx.compose.ui.test.performTextInput
import androidx.compose.ui.unit.toSize
import androidx.test.platform.app.InstrumentationRegistry
import org.bitcoinppl.cove.AppManager
import org.bitcoinppl.cove.test.HardwareSignerControl
import org.bitcoinppl.cove.test.LARGEST_SYSTEM_FONT_SCALE
import org.bitcoinppl.cove.test.WithFontScale
import org.bitcoinppl.cove.test.awaitOnMain
import org.bitcoinppl.cove.test.bootstrapRustRuntimeForUiTest
import org.bitcoinppl.cove.test.textLayoutProblem
import org.bitcoinppl.cove.ui.theme.CoveTheme
import org.bitcoinppl.cove_core.KeyTeleportManagerAction
import org.bitcoinppl.cove_core.KeyTeleportManagerState
import org.bitcoinppl.cove_core.KeyTeleportRoute
import org.junit.After
import org.junit.Assert.assertTrue
import org.junit.Assume.assumeTrue
import org.junit.Before
import org.junit.Rule
import org.junit.Test

/**
 * The received recovery words, private key and message reviews keep every text whole and every
 * action reachable at the largest system font scale in the dark theme, before and after a reveal
 *
 * Hosts the real KeyTeleport receive screen. The UI test host plays the sending phone
 * ([KeyTeleportHostSender]) in place of the camera between two devices, since the app itself
 * never sends an xprv it holds no wallet for, or a message
 *
 * Each named text must lay out whole and, once scrolled to, lie whole inside the screen, and each
 * action must also take a click. Every other text on the screen must lay out whole too, so a text
 * the test doesn't name is checked as well. The reveal hint's own text is hidden from the
 * semantics tree, so only the reveal control around it is checked
 */
class KeyTeleportReviewLayoutTest {
    @get:Rule
    val compose = createAndroidComposeRule<ComponentActivity>()

    // set once setUp's guards pass, so a skipped test never ends a session it found
    private var ownsReceiveSession = false

    @Before
    fun setUp() {
        bootstrapRustRuntimeForUiTest()

        // receiving deletes the stored receive session afterwards, so never run over someone's pending transfer
        assumeTrue("a KeyTeleport receive session is already stored on this device", !hasStoredReceiveSession())
        HardwareSignerControl().requireHealthy()
        ownsReceiveSession = true

        compose.setContent {
            CoveTheme(darkTheme = true, dynamicColor = false) {
                WithFontScale(LARGEST_SYSTEM_FONT_SCALE) {
                    KeyTeleportContainer(app = AppManager.getInstance(), route = KeyTeleportRoute.RECEIVE)
                }
            }
        }
    }

    @After
    fun tearDown() {
        val app = AppManager.getInstance()

        try {
            // setUp found no stored session, so one here is this test's own
            if (ownsReceiveSession && hasStoredReceiveSession()) endReceive(app.getKeyTeleportManager())
        } finally {
            InstrumentationRegistry.getInstrumentation().runOnMainSync { app.clearKeyTeleportManager() }
        }
    }

    @Test
    fun mnemonicReviewFitsAtTheLargestFontScale() {
        receive(KeyTeleportHostPayload.Mnemonic(SENT_WORDS.joinToString(" ")))
        compose.waitUntil(STATE_TIMEOUT_MILLIS) { receiver().state is KeyTeleportManagerState.ReceiveMnemonicReview }
        compose.waitForIdle()

        assertTextsFit("mnemonic review", "Recovery words received", "Cove found a 12-word wallet. Review it below or import it directly.")
        assertControlFits("mnemonic review: reveal", hasContentDescription(RECOVERY_WORDS_HINT) and hasClickAction())
        assertActionsFit("mnemonic review", "Import Wallet", "Finish Without Importing")
        assertEveryTextDrawsWhole("mnemonic review")

        compose.onNodeWithContentDescription(RECOVERY_WORDS_HINT).performClick()
        compose.waitUntil(STATE_TIMEOUT_MILLIS) { compose.onAllNodes(hasText("1."), useUnmergedTree = true).fetchSemanticsNodes().isNotEmpty() }

        // the test words repeat, so each word is found inside its own numbered chip
        SENT_WORDS.forEachIndexed { index, word ->
            val position = index + 1
            val chip = compose.onNode(isChipFor(position, word), useUnmergedTree = false)
            assertShowsWhole(chip, "revealed words: word $position")

            val chipId = chip.fetchSemanticsNode("revealed words: chip $position is missing").id
            assertTextFits(compose.onNode(hasText("$position.") and isInside(chipId), useUnmergedTree = true), "revealed words: number $position")
            assertTextFits(compose.onNode(hasText(word) and isInside(chipId), useUnmergedTree = true), "revealed words: word $position")
        }

        assertEveryTextDrawsWhole("revealed words")
    }

    @Test
    fun xprvReviewFitsAtTheLargestFontScale() {
        receive(KeyTeleportHostPayload.Xprv(SENT_XPRV))
        compose.waitUntil(STATE_TIMEOUT_MILLIS) { receiver().state is KeyTeleportManagerState.ReceiveXprvReview }
        compose.waitForIdle()

        assertTextsFit(
            "xprv review",
            "Extended private key received",
            "Import this key as a hot wallet, or reveal it only when you are ready to handle it.",
        )
        assertActionsFit("xprv review", "Reveal", "Import Wallet", "Finish Without Importing")
        assertEveryTextDrawsWhole("xprv review")

        compose.onNodeWithText("Reveal").performClick()
        compose.waitUntil(STATE_TIMEOUT_MILLIS) { compose.onAllNodes(hasText(SENT_XPRV)).fetchSemanticsNodes().isNotEmpty() }
        assertTextsFit("revealed xprv", SENT_XPRV)
        assertActionsFit("revealed xprv", "Copy", "Hide")
        assertEveryTextDrawsWhole("revealed xprv")
    }

    @Test
    fun messageReviewFitsAtTheLargestFontScale() {
        receive(KeyTeleportHostPayload.Notes(SENT_NOTES))
        compose.waitUntil(STATE_TIMEOUT_MILLIS) { receiver().state is KeyTeleportManagerState.ReceiveMessageReview }
        compose.waitForIdle()

        assertTextsFit(
            "message review",
            "Messages received",
            "This transfer contains text, not a wallet. Cove displays it exactly as received.",
            "Quick Note",
            "Personal",
            "MESSAGE",
            "Attack at dawn.",
            "Exchange",
            "Work",
            "USERNAME",
            "satoshi",
            "PASSWORD",
            "hunter2",
            "WEBSITE",
            "example.com",
            "NOTES",
            "Rotate yearly",
        )
        assertActionsFit("message review", "Done")
        assertEveryTextDrawsWhole("message review")
    }

    // MARK: receiving

    /** Answers the screen's receiver request from the host and unlocks the response with its password */
    private fun receive(payload: KeyTeleportHostPayload) {
        val receiver = receiver()
        val response = KeyTeleportHostSender().send(payload, awaitRequest(receiver))
        val accepted =
            awaitOnMain("the receiver to read the sender response") {
                receiver.ingestKeyTeleportText(response.packetText, KeyTeleportFlowDirection.RECEIVE)
            }

        assertTrue("the receiver rejected the sender response", accepted)
        compose.waitUntil(STATE_TIMEOUT_MILLIS) { receiver.state is KeyTeleportManagerState.ReceiveEnterPassword }
        compose.onNode(hasSetTextAction()).performTextInput(response.password)
        compose.onNodeWithText("Continue").performClick()
    }

    private fun receiver(): KeyTeleportManager = awaitOnMain("the screen's KeyTeleport manager") { AppManager.getInstance().getKeyTeleportManager() }

    // MARK: layout checks

    private fun assertTextsFit(
        screen: String,
        vararg texts: String,
    ) {
        texts.forEach { text ->
            val node = compose.onNode(hasText(text) and hasTextLayout(), useUnmergedTree = true)
            assertShowsWhole(node, "$screen: \"${text.take(TEXT_IN_MESSAGE)}\"")
            assertTextFits(node, "$screen: \"${text.take(TEXT_IN_MESSAGE)}\"")
        }
    }

    /** Each action must show whole once scrolled to, take a click, be enabled, and lay its label out whole */
    private fun assertActionsFit(
        screen: String,
        vararg labels: String,
    ) {
        labels.forEach { label ->
            assertControlFits("$screen: \"$label\"", hasText(label) and hasClickAction())
            assertTextFits(compose.onNode(hasText(label) and hasTextLayout(), useUnmergedTree = true), "$screen: \"$label\" label")
        }
    }

    private fun assertControlFits(
        name: String,
        matcher: SemanticsMatcher,
    ) {
        val control = compose.onNode(matcher, useUnmergedTree = false)
        assertShowsWhole(control, name)
        control.assertIsEnabled()
    }

    /**
     * Scrolls to the node as a user would, then fails unless it lies inside the screen's width
     * and shows whole there, not cut off by the scroll area or the screen's edge
     *
     * `boundsInRoot` is clipped by every ancestor, while the node's position and size are not
     */
    private fun assertShowsWhole(
        node: SemanticsNodeInteraction,
        name: String,
    ) {
        node.performScrollTo()
        compose.waitForIdle()

        val semantics = node.fetchSemanticsNode("$name is missing")
        val whole = Rect(semantics.positionInRoot, semantics.size.toSize())
        val shown = semantics.boundsInRoot
        val width = compose.onRoot().fetchSemanticsNode().size.width

        assertTrue(
            "$name runs past the side of the screen at $whole in a ${width}px wide screen",
            whole.left >= -CLIP_TOLERANCE_PX && whole.right <= width + CLIP_TOLERANCE_PX,
        )
        assertTrue(
            "$name shows only $shown of $whole",
            whole.left >= shown.left - CLIP_TOLERANCE_PX &&
                whole.top >= shown.top - CLIP_TOLERANCE_PX &&
                whole.right <= shown.right + CLIP_TOLERANCE_PX &&
                whole.bottom <= shown.bottom + CLIP_TOLERANCE_PX,
        )
    }

    private fun assertTextFits(
        node: SemanticsNodeInteraction,
        name: String,
    ) {
        val semantics = node.fetchSemanticsNode("$name is missing")
        val problem = compose.runOnIdle { semantics.textLayoutProblem() }
        assertTrue("$name doesn't draw whole: $problem", problem == null)
    }

    /**
     * Every text in the screen, named or not, must lay out whole; text fields scroll their own entry by design
     *
     * Texts are found by their text rather than their layout action, so one that stops reporting
     * its layout fails instead of dropping out of the check
     */
    private fun assertEveryTextDrawsWhole(screen: String) {
        val texts =
            compose
                .onAllNodes(
                    SemanticsMatcher.keyIsDefined(SemanticsProperties.Text) and
                        !SemanticsMatcher.keyIsDefined(SemanticsProperties.EditableText),
                    useUnmergedTree = true,
                ).fetchSemanticsNodes()

        assertTrue("$screen has no texts to check", texts.isNotEmpty())
        val problems =
            compose.runOnIdle {
                texts.mapNotNull { node ->
                    val text = node.config.getOrNull(SemanticsProperties.Text).orEmpty().joinToString(" ") { it.text }
                    node.textLayoutProblem()?.let { "\"${text.take(TEXT_IN_MESSAGE)}\": $it" }
                }
            }

        assertTrue("$screen has texts that don't draw whole: $problems", problems.isEmpty())
    }

    private fun hasTextLayout(): SemanticsMatcher = SemanticsMatcher.keyIsDefined(SemanticsActions.GetTextLayoutResult)

    // a merged node keeps the id of the node it merges into, so unmerged nodes find it among their parents
    private fun isInside(ancestorId: Int): SemanticsMatcher =
        SemanticsMatcher("is inside node $ancestorId") { node ->
            generateSequence(node.parent) { it.parent }.any { it.id == ancestorId }
        }

    // a chip merges its number and word into one node
    private fun isChipFor(
        position: Int,
        word: String,
    ): SemanticsMatcher =
        SemanticsMatcher("is the chip for word $position") { node ->
            node.config.getOrNull(SemanticsProperties.Text).orEmpty().map { it.text } == listOf("$position.", word)
        }

    private companion object {
        const val STATE_TIMEOUT_MILLIS = 15_000L
        const val RECOVERY_WORDS_HINT = "Tap to reveal recovery words"

        // Compose positions are floats, and a box rounds its edges to whole pixels
        const val CLIP_TOLERANCE_PX = 1f

        // enough of a text to tell which one a failure names
        const val TEXT_IN_MESSAGE = 40

        // a BIP39 test vector, never used for funds; its repeated words check each chip by position
        val SENT_WORDS =
            listOf(
                "legal", "winner", "thank", "year", "wave", "sausage",
                "worth", "useful", "legal", "winner", "thank", "yellow",
            )

        // the master key of BIP32 test vector 1
        const val SENT_XPRV =
            "xprv9s21ZrQH143K3QTDL4LXw2F7HEK3wJUD2nW2nRk4stbPy6cq3jPPqjiChkVvvNKmPGJxWUtg6LnF5kejMRNNU3TGtRBeJgk33yuGBxrMPHi"

        // a note and a password record in the shape COLDCARD's Secure Notes send them
        val SENT_NOTES =
            listOf(
                mapOf("title" to "Quick Note", "misc" to "Attack at dawn.", "group" to "Personal"),
                mapOf(
                    "title" to "Exchange",
                    "user" to "satoshi",
                    "password" to "hunter2",
                    "site" to "example.com",
                    "misc" to "Rotate yearly",
                    "group" to "Work",
                ),
            )
    }
}
