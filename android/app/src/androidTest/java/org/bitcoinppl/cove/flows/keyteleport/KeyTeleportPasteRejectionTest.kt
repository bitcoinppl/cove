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
        compose.setContent {
            CoveTheme(dynamicColor = false) {
                KeyTeleportContainer(app = AppManager.getInstance(), route = KeyTeleportRoute.SEND)
            }
        }

        putOnClipboard(MAINNET_ADDRESS)
        compose.onNodeWithText("Paste").performClick()

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
        // a recognisable non-KeyTeleport payload, like the PSBT in the report
        const val MAINNET_ADDRESS = "bc1qar0srrr7xfkvy5l643lydnw9re59gtzzwf5mdq"
    }
}
