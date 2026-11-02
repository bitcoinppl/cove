package org.bitcoinppl.cove.flows.settings

import android.content.ClipData
import android.content.ClipDescription
import android.content.ClipboardManager
import android.content.Context
import android.os.Build
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import org.bitcoinppl.cove.CoveApplication
import org.bitcoinppl.cove.ui.theme.CoveTheme
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

/**
 * Copying the private key goes through the real ClipboardManager: the clip must be marked sensitive
 * and cleared once its lifetime ends, while text copied over it must survive that clear
 *
 * Lives in the UI test flavour, whose application shortens the clip lifetime from a minute to a
 * few seconds
 */
@RunWith(AndroidJUnit4::class)
class SensitiveClipboardCopyTest {
    @get:Rule
    val compose = createComposeRule()

    private val context: Context = InstrumentationRegistry.getInstrumentation().targetContext
    private val clipboard = context.getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager
    private val lifetimeMillis = (context.applicationContext as CoveApplication).sensitiveClipboardLifetimeMillis

    @Before
    fun showPrivateKey() {
        compose.setContent {
            CoveTheme(dynamicColor = false) {
                XprvRevealDialog(xprv = TEST_XPRV, onDismiss = {})
            }
        }
    }

    @After
    fun clearClipboard() {
        runCatching { clipboard.clearPrimaryClip() }
    }

    @Test
    fun copiedPrivateKeyIsMarkedSensitiveAndClearedWhenItsLifetimeEnds() {
        compose.onNodeWithText("Copy").performClick()
        compose.waitForIdle()

        assertEquals("the private key was not copied", TEST_XPRV, clipText())
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
            val extras = clipboard.primaryClipDescription?.extras
            assertTrue(
                "the copied private key is not marked sensitive",
                extras?.getBoolean(ClipDescription.EXTRA_IS_SENSITIVE) == true,
            )
        }

        compose.waitUntil(timeoutMillis = lifetimeMillis + CLEAR_MARGIN_MILLIS) { clipText() == null }
    }

    @Test
    fun textCopiedOverThePrivateKeySurvivesItsLifetime() {
        compose.onNodeWithText("Copy").performClick()
        compose.waitForIdle()
        assertEquals("the private key was not copied", TEST_XPRV, clipText())

        InstrumentationRegistry.getInstrumentation().runOnMainSync {
            clipboard.setPrimaryClip(ClipData.newPlainText("replacement", REPLACEMENT_TEXT))
        }

        // the expiry runs on the main looper once the lifetime ends, so this sleep outlasts it
        Thread.sleep(lifetimeMillis + CLEAR_MARGIN_MILLIS)
        compose.waitForIdle()

        assertEquals("the private key expiry cleared text copied after it", REPLACEMENT_TEXT, clipText())
    }

    private fun clipText(): String? =
        clipboard.primaryClip
            ?.takeIf { it.itemCount > 0 }
            ?.getItemAt(0)
            ?.text
            ?.toString()

    private companion object {
        const val CLEAR_MARGIN_MILLIS = 3_000L
        const val REPLACEMENT_TEXT = "copied after the private key"

        // BIP32 test vector 1 master key, published and never used for funds
        const val TEST_XPRV =
            "xprv9s21ZrQH143K3QTDL4LXw2F7HEK3wJUD2nW2nRk4stbPy6cq3jPPqjiChkVvvNKmPGJxWUtg6LnF5kejMRNNU3TGtRBeJgk33yuGBxrMPHi"
    }
}
