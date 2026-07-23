package org.bitcoinppl.cove.flows.keyteleport

import androidx.compose.material3.Text
import androidx.compose.ui.test.assertHasClickAction
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.test.ext.junit.runners.AndroidJUnit4
import org.bitcoinppl.cove.ui.theme.CoveTheme
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

@RunWith(AndroidJUnit4::class)
class KeyTeleportRevealPairTest {
    @get:Rule
    val compose = createComposeRule()

    @Test
    fun revealActionSwapsTheOnlyExposedValue() {
        compose.setContent {
            CoveTheme(dynamicColor = false) {
                KeyTeleportRevealPair(
                    qrHint = "Tap to show QR code",
                    codeHint = "Tap to show password",
                    qr = { Text("QR payload") },
                    code = { Text("Secret password") },
                )
            }
        }

        compose.onNodeWithText("QR payload").assertIsDisplayed()
        compose.onNodeWithText("Secret password").assertDoesNotExist()

        compose
            .onNodeWithContentDescription("Tap to show password")
            .assertHasClickAction()
            .performClick()

        compose.onNodeWithText("Secret password").assertIsDisplayed()
        compose.onNodeWithText("QR payload").assertDoesNotExist()

        compose
            .onNodeWithContentDescription("Tap to show QR code")
            .assertHasClickAction()
            .performClick()

        compose.onNodeWithText("QR payload").assertIsDisplayed()
        compose.onNodeWithText("Secret password").assertDoesNotExist()
    }
}
