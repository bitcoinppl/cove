package org.bitcoinppl.cove.views

import androidx.activity.ComponentActivity
import androidx.compose.material3.Button
import androidx.compose.material3.Text
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.test.hasClickAction
import androidx.compose.ui.test.hasTestTag
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.test.platform.app.InstrumentationRegistry
import org.bitcoinppl.cove.Auth
import org.bitcoinppl.cove.test.bootstrapRustRuntimeForUiTest
import org.bitcoinppl.cove.ui.theme.CoveTheme
import org.bitcoinppl.cove_core.AppLockReason
import org.bitcoinppl.cove_core.AuthManagerAction
import org.bitcoinppl.cove_core.AuthType
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Assume.assumeTrue
import org.junit.Before
import org.junit.Rule
import org.junit.Test

/**
 * While the app is locked, nothing under the PIN cover may be readable or actionable through
 * accessibility
 *
 * The cover only blocks touches and the screen under it stays composed, so without clearing its
 * semantics a screen reader or any accessibility service could read secrets behind the cover and
 * invoke their reveal actions. Accessibility services build their view from these Compose
 * semantics, so this checks the semantics the cover clears, not TalkBack itself
 */
class LockViewAccessibilityTest {
    @get:Rule
    val compose = createAndroidComposeRule<ComponentActivity>()

    private val instrumentation = InstrumentationRegistry.getInstrumentation()
    private var pinSet = false

    @Before
    fun setUp() {
        bootstrapRustRuntimeForUiTest()

        // the test sets and removes its own PIN, so never run it over a credential someone relies on
        assumeTrue("an app lock is already set up on this device", Auth.type == AuthType.NONE)

        Auth.dispatch(AuthManagerAction.SetPin(TEST_PIN))
        pinSet = true
        compose.waitUntil(STATE_TIMEOUT_MILLIS) { Auth.type == AuthType.PIN }
    }

    @After
    fun tearDown() {
        if (!pinSet) return

        // with the PIN gone, Rust lets the lock clear without authenticating
        Auth.dispatch(AuthManagerAction.DisablePin)
        instrumentation.runOnMainSync { Auth.unlockIfAuthDisabled() }
    }

    @Test
    fun contentUnderThePinCoverIsHiddenFromAccessibilityUntilUnlocked() {
        var clicks = 0
        compose.setContent {
            CoveTheme(dynamicColor = false) {
                LockView {
                    Button(onClick = { clicks++ }, modifier = Modifier.testTag(PROTECTED_TAG)) {
                        Text(PROTECTED_LABEL)
                    }
                }
            }
        }

        // unlocked, the content is reachable, so the locked checks below are not vacuous
        if (Auth.isLocked) enterPin(TEST_PIN)
        compose.waitUntil(STATE_TIMEOUT_MILLIS) { !Auth.isLocked }
        compose.waitForIdle()
        assertEquals(1, readableNodeCount())
        assertEquals(1, actionableNodeCount())

        instrumentation.runOnMainSync { Auth.lock(AppLockReason.REAUTHENTICATION) }
        compose.waitForIdle()
        assertTrue("the app did not lock", Auth.isLocked)
        compose.onNodeWithText(PIN_TITLE).assertExists()
        assertEquals("content under the PIN cover was readable", 0, readableNodeCount())
        assertEquals("content under the PIN cover was actionable", 0, actionableNodeCount())
        assertEquals(0, clicks)

        enterPin(TEST_PIN)
        compose.waitUntil(STATE_TIMEOUT_MILLIS) { !Auth.isLocked }
        compose.waitForIdle()
        assertEquals("content stayed hidden after unlocking", 1, readableNodeCount())
        assertEquals("content stayed inert after unlocking", 1, actionableNodeCount())
    }

    // unlock the way a user does, through the PIN pad on the cover
    private fun enterPin(pin: String) {
        pin.forEach { digit -> compose.onNodeWithTag("numberPadPin.digit.$digit").performClick() }
    }

    // text and actions are checked apart, so dropping only one of them from the cover still fails
    private fun readableNodeCount(): Int =
        compose
            .onAllNodes(hasText(PROTECTED_LABEL), useUnmergedTree = false)
            .fetchSemanticsNodes()
            .size

    private fun actionableNodeCount(): Int =
        compose
            .onAllNodes(hasTestTag(PROTECTED_TAG) and hasClickAction(), useUnmergedTree = false)
            .fetchSemanticsNodes()
            .size

    private companion object {
        const val TEST_PIN = "864213"
        const val PIN_TITLE = "Enter Cove PIN"
        const val PROTECTED_LABEL = "Content behind the lock"
        const val PROTECTED_TAG = "lockViewAccessibility.protected"
        const val STATE_TIMEOUT_MILLIS = 15_000L
    }
}
