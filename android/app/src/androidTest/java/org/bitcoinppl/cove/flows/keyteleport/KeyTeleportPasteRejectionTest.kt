package org.bitcoinppl.cove.flows.keyteleport

import android.content.ClipData
import android.content.ClipboardManager
import android.content.Context
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.hasAnyAncestor
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.isDialog
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import org.bitcoinppl.cove.AppManager
import org.bitcoinppl.cove.test.bootstrapRustRuntimeForUiTest
import org.bitcoinppl.cove.ui.theme.CoveTheme
import org.bitcoinppl.cove_core.KeyTeleportRoute
import org.bitcoinppl.cove_core.MultiFormatException
import org.bitcoinppl.cove_core.StringOrData
import org.junit.Assert.assertThrows
import org.junit.Assert.assertTrue
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

        assertRejectionAsksForTheReceiverRequest()
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

    private fun putOnClipboard(text: String) {
        val instrumentation = InstrumentationRegistry.getInstrumentation()
        instrumentation.runOnMainSync {
            val clipboard =
                instrumentation.targetContext.getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager
            clipboard.setPrimaryClip(ClipData.newPlainText("test", text))
        }
    }

    private companion object {
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
