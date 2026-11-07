package org.bitcoinppl.cove.flows.keyteleport

import android.content.ClipData
import android.content.ClipboardManager
import android.content.Context
import android.graphics.Bitmap
import androidx.compose.ui.geometry.Rect
import androidx.compose.ui.semantics.SemanticsNode
import androidx.compose.ui.test.SemanticsMatcher
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.assertIsEnabled
import androidx.compose.ui.test.hasAnyAncestor
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.isDialog
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.onRoot
import androidx.compose.ui.test.performClick
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import org.bitcoinppl.cove.AppManager
import org.bitcoinppl.cove.test.assertTextsMatchTheirReferences
import org.bitcoinppl.cove.test.bootstrapRustRuntimeForUiTest
import org.bitcoinppl.cove.test.matchTextReference
import org.bitcoinppl.cove.test.saveNodeScreenshotToLayoutAudit
import org.bitcoinppl.cove.test.textLabel
import org.bitcoinppl.cove.test.textLayouts
import org.bitcoinppl.cove.ui.theme.CoveTheme
import org.bitcoinppl.cove_core.KeyTeleportRoute
import org.bitcoinppl.cove_core.MultiFormatException
import org.bitcoinppl.cove_core.StringOrData
import org.junit.Assert.assertEquals
import org.junit.Assert.assertThrows
import org.junit.Assert.assertTrue
import org.junit.Assert.fail
import org.junit.Before
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

@RunWith(AndroidJUnit4::class)
class KeyTeleportPasteRejectionTest {
    @get:Rule
    val compose = createComposeRule()

    @Before
    fun bootstrapRustRuntime() {
        bootstrapRustRuntimeForUiTest()
    }

    // regression for #920: the sender waits for a receiver request, so pointing it at a
    // "response" sends the user to the wrong device screen
    @Test
    fun senderRejectsANonKeyTeleportPasteByAskingForTheReceiverRequest() {
        pasteIntoSender(MAINNET_ADDRESS)

        assertRejectionAsksForTheReceiverRequest()
    }

    // the 1.4.0 report pasted PSBT text into the sender and was told to paste a response
    @Test
    fun senderRejectsARawPsbtPasteByAskingForTheReceiverRequest() {
        // guards the fixture, it must reach the sender as a PSBT and not as some other format
        assertThrows(MultiFormatException.PsbtNotSigned::class.java) {
            StringOrData.String(BIP174_UNSIGNED_PSBT_BASE64).tryIntoMultiFormat()
        }

        pasteIntoSender(BIP174_UNSIGNED_PSBT_BASE64)

        // the protected screen gives adb no screencap, so the test captures the error and the
        // screen after OK from inside the app, where a person used to photograph the phone
        assertRejectionAsksForTheReceiverRequest()
        val capture = saveNodeScreenshotToLayoutAudit(compose.onNode(isDialog()), "s17-sender-psbt-error.png")
        assertCaptureShowsContent(capture)
        assertDialogTextFitsInsideTheDialog(capture)
        assertOkLeavesTheSenderReadyForAnotherPaste(BIP174_UNSIGNED_PSBT_BASE64)
    }

    // a KeyTeleport PSBT packet is the one PSBT shape Rust recognises as KeyTeleport, through
    // its own unsupported error, so it must still name the request the sender is waiting for
    @Test
    fun senderRejectsAKeyTeleportPsbtPacketPasteByAskingForTheReceiverRequest() {
        assertThrows(MultiFormatException.KeyTeleportPsbtNotSupported::class.java) {
            StringOrData.String(KEY_TELEPORT_PSBT_PACKET).tryIntoMultiFormat()
        }

        pasteIntoSender(KEY_TELEPORT_PSBT_PACKET)

        assertRejectionAsksForTheReceiverRequest()
    }

    private fun pasteIntoSender(text: String) {
        compose.setContent {
            CoveTheme(dynamicColor = false) {
                KeyTeleportContainer(app = AppManager.getInstance(), route = KeyTeleportRoute.SEND)
            }
        }

        putOnClipboard(text)
        compose.onNodeWithText("Paste").performClick()
    }

    private fun assertRejectionAsksForTheReceiverRequest() {
        // the screen header already says "request", so only the rejection dialog counts
        val inDialog = hasAnyAncestor(isDialog())
        compose
            .onNode(inDialog and hasText("request", substring = true, ignoreCase = true))
            .assertIsDisplayed()

        val mentionsResponse =
            compose
                .onAllNodes(inDialog and hasText("response", substring = true, ignoreCase = true))
                .fetchSemanticsNodes()
        assertTrue("sender rejection still asks for a response", mentionsResponse.isEmpty())
    }

    // the release checklist asked for a screenshot of the screen after OK, to see that the sender
    // is still usable and not stuck behind the error
    private fun assertOkLeavesTheSenderReadyForAnotherPaste(rejected: String) {
        compose.onNode(hasAnyAncestor(isDialog()) and hasText(DIALOG_OK)).performClick()
        compose.waitForIdle()

        val dialogs = compose.onAllNodes(isDialog()).fetchSemanticsNodes()
        assertTrue("the rejection dialog stayed open after OK", dialogs.isEmpty())
        compose.onNodeWithText("Paste").assertIsDisplayed().assertIsEnabled()
        val capture = saveNodeScreenshotToLayoutAudit(compose.onRoot(), "s17-sender-after-psbt-error.png")
        assertCaptureShowsContent(capture)
        assertSenderTextIsDrawnAsLaidOut(capture)

        // pasting again reaches the sender and is rejected the same way, rather than doing nothing
        putOnClipboard(rejected)
        compose.onNodeWithText("Paste").performClick()
        assertRejectionAsksForTheReceiverRequest()
    }

    /**
     * Clipped, truncated, covered, overlapping, undrawn or faint text is what a screenshot of the
     * error would have shown, so each text node must draw in the dialog's [capture] as a reference
     * rendering of its own layout does, keep its full laid-out size, and stay inside the dialog
     *
     * The title, the whole message and OK are looked up by name first, since a missing text, or one
     * that lays out nothing, would otherwise leave nothing to compare
     */
    private fun assertDialogTextFitsInsideTheDialog(capture: Bitmap) {
        val dialog = compose.onNode(isDialog()).fetchSemanticsNode().boundsInWindow
        val inDialog = hasAnyAncestor(isDialog())
        val required =
            listOf(DIALOG_TITLE, KeyTeleportFlowDirection.SEND.rejectedInputMessage, DIALOG_OK).map {
                requiredText("the rejection dialog", inDialog, it)
            }

        val texts = (required + drawnTexts(inDialog)).distinctBy { it.id }

        assertTextsMatchTheirReferences(
            "the rejection dialog",
            texts.map { capture.matchTextReference(it, dialog.topLeft, compose.density) },
        )

        texts.forEach { node ->
            val label = node.textLabel()
            val bounds = node.boundsInWindow
            assertTrue("\"$label\" has no size", bounds.width > 0f && bounds.height > 0f)
            assertTrue("\"$label\" reaches outside the dialog", dialog.contains(bounds))
            node.overflow()?.let { reason -> fail("\"$label\" is cut off: $reason") }

            // boundsInWindow is already clipped by every parent, so a clipped node shows less than its layout size
            assertTrue(
                "\"$label\" is clipped by its container",
                bounds.width >= node.size.width - CLIP_TOLERANCE_PX && bounds.height >= node.size.height - CLIP_TOLERANCE_PX,
            )
        }

        texts.forEachIndexed { index, node ->
            texts.drop(index + 1).forEach { other ->
                assertTrue("dialog texts overlap", !node.boundsInWindow.overlaps(other.boundsInWindow))
            }
        }
    }

    // once the dialog closes the sender is the only window, and every text on it must still be drawn
    // whole; its title and instructions are looked up by name first, since without them the header
    // and buttons alone would still give the check something to compare
    private fun assertSenderTextIsDrawnAsLaidOut(capture: Bitmap) {
        val root = compose.onRoot().fetchSemanticsNode().boundsInWindow
        val anywhere = SemanticsMatcher("anywhere") { true }
        val required = listOf(SENDER_TITLE, SENDER_INSTRUCTIONS).map { requiredText("the sender after OK", anywhere, it) }
        val texts = (required + drawnTexts(anywhere)).distinctBy { it.id }

        assertTextsMatchTheirReferences(
            "the sender after OK",
            texts.map { capture.matchTextReference(it, root.topLeft, compose.density) },
        )
    }

    // the one node in [matcher] showing exactly [label], which must lay out that text
    private fun requiredText(
        where: String,
        matcher: SemanticsMatcher,
        label: String,
    ): SemanticsNode {
        val nodes = compose.onAllNodes(matcher and hasText(label), useUnmergedTree = true).fetchSemanticsNodes()
        assertEquals("$where shows \"$label\" the wrong number of times", 1, nodes.size)

        val node = nodes.single()
        assertEquals("\"$label\" lays out other text than it shows", label, node.textLayouts().singleOrNull()?.layoutInput?.text?.text)
        return node
    }

    // text nodes that lay out visible characters, leaving out an empty field
    private fun drawnTexts(matcher: SemanticsMatcher): List<SemanticsNode> =
        compose
            .onAllNodes(matcher, useUnmergedTree = true)
            .fetchSemanticsNodes()
            .filter { node -> node.textLayouts().any { it.layoutInput.text.isNotBlank() } }

    // a protected window can come back blank, which would make the saved capture worthless
    private fun assertCaptureShowsContent(bitmap: Bitmap) {
        val colors = mutableSetOf<Int>()
        for (y in 0 until bitmap.height step CAPTURE_SAMPLE_STEP) {
            for (x in 0 until bitmap.width step CAPTURE_SAMPLE_STEP) {
                colors += bitmap.getPixel(x, y)
            }
        }

        assertTrue("the capture is a single color", colors.size > 1)
    }

    private fun Rect.contains(other: Rect): Boolean =
        other.left >= left && other.top >= top && other.right <= right && other.bottom <= bottom

    // the paragraph width is the width the text was offered, not the width it drew, so a short title
    // in a wide dialog looked overflowing to hasVisualOverflow on CI; measure the widest drawn line
    private fun SemanticsNode.overflow(): String? {
        return textLayouts().firstNotNullOfOrNull { layout ->
            val paragraph = layout.multiParagraph
            val widestLine = (0 until paragraph.lineCount).maxOfOrNull { paragraph.getLineRight(it) - paragraph.getLineLeft(it) } ?: 0f

            when {
                paragraph.didExceedMaxLines -> "it has more lines than it may show"
                widestLine - layout.size.width > CLIP_TOLERANCE_PX ->
                    "its widest line is ${widestLine}px in a ${layout.size.width}px layout"
                paragraph.height - layout.size.height > CLIP_TOLERANCE_PX ->
                    "its text is ${paragraph.height}px tall in a ${layout.size.height}px layout"
                else -> null
            }
        }
    }

    private fun putOnClipboard(text: String) {
        val instrumentation = InstrumentationRegistry.getInstrumentation()
        instrumentation.runOnMainSync {
            val clipboard =
                instrumentation.targetContext.getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager
            clipboard.setPrimaryClip(ClipData.newPlainText("test", text))
        }
    }

    private companion object {
        // every fourth pixel is enough to tell a drawn screen from a blank one
        const val CAPTURE_SAMPLE_STEP = 4

        // the title and button KeyTeleportMessageDialog shows over every message
        const val DIALOG_TITLE = "KeyTeleport"
        const val DIALOG_OK = "OK"

        // the title and instructions SendIdleView shows above Scan and Paste
        const val SENDER_TITLE = "Send a wallet"
        const val SENDER_INSTRUCTIONS = "Scan or paste the receiver code, then choose a hot wallet to send."

        // bounds are floats from the same layout, so allow rounding but not a clipped line of text
        const val CLIP_TOLERANCE_PX = 1f

        // a recognisable non-KeyTeleport payload
        const val MAINNET_ADDRESS = "bc1qar0srrr7xfkvy5l643lydnw9re59gtzzwf5mdq"

        // the unsigned BIP 174 test vector, the same PSBT as TEST_PSBT_HEX in the Rust tests
        const val BIP174_UNSIGNED_PSBT_BASE64 =
            "cHNidP8BAJoCAAAAAljoeiG1ba8MI76OcHBFbDNvfLqlyHV5JPVFiHuyq911AAAAAAD/////g40EJ9DsZQpoqka7" +
                "CwmK6kQiwHGyyng1Kgd5WdB86h0BAAAAAP////8CcKrwCAAAAAAWABTYXCtx0AYLCcmIauuBXlCZHdoSTQDh9QUA" +
                "AAAAFgAUAK6pouXw+HaliN9VRuh0LR2HAI8AAAAAAAAAAAA="

        // a single-part BBQr KeyTeleport PSBT packet (type E) carrying the same BIP 174 PSBT
        const val KEY_TELEPORT_PSBT_PACKET =
            "B\$2E0100OBZWE5H7AEAJUAQAAAAAEWHIPIQ3K3NPBQR35DTQOBCWYM3PPS5KLSDVPESPKRMIPOZKXXLVAAAAAAAA" +
                "7777774DRUCCPUHMMUFGRKSGXMFQTCXKIQRMA4NSZJ4DKKQHPFM5A7HKDUAQAAAAAD777777AJYKV4AIAAAAAAAW" +
                "AAKNQXBLOHIAMCYJZGEGV24BLZIJSHO2CJGQBYPVAUAAAAAACYABIAFOVGROL4HYO2SYRX2VI3UHILI5Q4AI6AAA" +
                "AAAAAAAAAAAA"
    }
}
