@file:Suppress("PackageNaming")

package org.bitcoinppl.cove.flows.SettingsFlow

import androidx.compose.ui.semantics.SemanticsNode
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.semantics.getOrNull
import androidx.compose.ui.test.SemanticsMatcher
import androidx.compose.ui.test.hasAnyAncestor
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.isDialog
import androidx.compose.ui.test.isToggleable
import androidx.compose.ui.test.junit4.ComposeTestRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performScrollTo
import androidx.test.platform.app.InstrumentationRegistry
import org.bitcoinppl.cove.App
import org.bitcoinppl.cove.Auth
import org.bitcoinppl.cove.test.bootstrapRustRuntimeForUiTest
import org.bitcoinppl.cove.test.coveComposeRule
import org.bitcoinppl.cove.test.restoreNoAppLock
import org.bitcoinppl.cove.ui.theme.CoveTheme
import org.bitcoinppl.cove_core.AuthManagerAction
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Before
import org.junit.Rule
import org.junit.Test

/**
 * Choosing the app PIN as the decoy PIN in Settings shows the reason in English
 *
 * The alert used to show the Kotlin message of the structured UniFFI error, a field dump such as
 * `v1=org.bitcoinppl.cove_core.TrickPinException$SameAsCurrentPin:`
 */
class DecoyPinSameAsCurrentPinAlertTest {
    @get:Rule
    val compose = coveComposeRule()

    private val instrumentation = InstrumentationRegistry.getInstrumentation()

    @Before
    fun enablePin() {
        bootstrapRustRuntimeForUiTest()
        instrumentation.runOnMainSync { Auth.dispatch(AuthManagerAction.SetPin(PIN)) }
        compose.waitUntil(timeoutMillis = 5_000) { Auth.isAuthEnabled }
    }

    @After
    fun removeAppLock() {
        restoreNoAppLock()
    }

    @Test
    fun decoyPinSameAsTheAppPinShowsTheReasonInEnglish() {
        compose.setContent {
            CoveTheme {
                MainSettingsScreen(app = App)
            }
        }

        compose.onNodeWithText(DECOY_TOGGLE_TITLE).performScrollTo()
        compose.onNode(switchInRowOf(DECOY_TOGGLE_TITLE)).performClick()
        compose.onNodeWithText("Yes, Enable Decoy PIN").performClick()

        compose.awaitText("Enter Decoy PIN")
        compose.typePin(PIN)
        compose.awaitText("Confirm Decoy PIN")
        compose.typePin(PIN)
        compose.awaitText(ALERT_TITLE)

        // everything the alert shows besides its title and button, so a failure prints what the user saw
        val shownMessage =
            compose
                .onAllNodes(hasAnyAncestor(isDialog()))
                .fetchSemanticsNodes()
                .flatMap { node -> node.config.getOrNull(SemanticsProperties.Text).orEmpty().map { it.text } }
                .filterNot { it == ALERT_TITLE || it == "OK" }
                .joinToString("\n")

        assertFalse("the alert shows a UniFFI field dump: \"$shownMessage\"", shownMessage.contains("v1="))
        assertEquals(EXPECTED_MESSAGE, shownMessage)
    }

    // the switch is the row's trailing content, not a semantic child of its title, so it is the
    // toggle laid out level with the title
    private fun switchInRowOf(title: String): SemanticsMatcher {
        val titleBounds = compose.onNodeWithText(title).fetchSemanticsNode().boundsInRoot
        val switch: SemanticsNode =
            compose
                .onAllNodes(isToggleable())
                .fetchSemanticsNodes()
                .single { it.boundsInRoot.center.y in titleBounds.top..titleBounds.bottom }

        return SemanticsMatcher("the switch in the \"$title\" row") { it.id == switch.id }
    }

    private companion object {
        const val PIN = "135790"
        const val DECOY_TOGGLE_TITLE = "Enable Decoy PIN"
        const val ALERT_TITLE = "Something went wrong!"
        const val EXPECTED_MESSAGE = "Unable to set the decoy PIN, because it is the same as your current PIN"
    }
}

private fun ComposeTestRule.awaitText(text: String) {
    waitUntil(timeoutMillis = 5_000) { onAllNodes(hasText(text)).fetchSemanticsNodes().isNotEmpty() }
}

// the pad only checks a full PIN after a short delay, which the callers wait out with awaitText
private fun ComposeTestRule.typePin(pin: String) {
    pin.forEach { digit -> onNodeWithTag("numberPadPin.digit.$digit").performClick() }
}
