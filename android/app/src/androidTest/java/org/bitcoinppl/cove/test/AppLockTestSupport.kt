package org.bitcoinppl.cove.test

import androidx.compose.ui.test.ComposeTimeoutException
import androidx.compose.ui.test.isDisplayed
import androidx.compose.ui.test.junit4.ComposeTestRule
import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.performClick
import androidx.test.platform.app.InstrumentationRegistry
import org.bitcoinppl.cove.AuthManager
import org.bitcoinppl.cove_core.AuthManagerAction
import org.bitcoinppl.cove_core.AuthType
import org.bitcoinppl.cove_core.BootstrapStep
import org.bitcoinppl.cove_core.RustAuthManager
import org.bitcoinppl.cove_core.bootstrapProgress

/**
 * Returns the app lock to a fresh install's state: no PIN, decoy PIN, wipe data PIN or biometric
 * unlock, the main wallets showing, and the app unlocked
 *
 * Each test runs in its own process, but every test shares the app's database, so a lock one test
 * leaves behind makes later tests that refuse to run over an existing app lock skip themselves.
 * Call it from `@After` so it also runs when the test fails
 */
fun restoreNoAppLock() {
    // a process that never finished bootstrap never opened the database, so it changed no lock
    if (bootstrapProgress() != BootstrapStep.COMPLETE) return

    var isInDecoyMode = true
    var isLocked = true
    InstrumentationRegistry.getInstrumentation().runOnMainSync {
        val auth = AuthManager.getInstance()

        // a main PIN unlock leaves decoy mode, which removing the PINs alone would not
        if (auth.isInDecoyMode()) auth.switchToMainMode()

        auth.dispatch(AuthManagerAction.DisableDecoyPin)
        auth.dispatch(AuthManagerAction.DisableWipeDataPin)
        auth.dispatch(AuthManagerAction.DisablePin)
        auth.dispatch(AuthManagerAction.DisableBiometric)
        auth.unlockIfAuthDisabled()

        isInDecoyMode = auth.isInDecoyMode()
        isLocked = auth.isLocked
    }

    // read the stored settings, since deleting a trick PIN only logs a database error
    val (authType, hasDecoyPin, hasWipeDataPin) =
        RustAuthManager().use { rust ->
            Triple(rust.authType(), rust.isDecoyPinEnabled(), rust.isWipeDataPinEnabled())
        }

    check(authType == AuthType.NONE && !hasDecoyPin && !hasWipeDataPin && !isInDecoyMode && !isLocked) {
        "the app lock is still set up after the test: authType=$authType decoyPin=$hasDecoyPin " +
            "wipeDataPin=$hasWipeDataPin decoyMode=$isInDecoyMode locked=$isLocked"
    }
}

/**
 * Types [pin] on a LockView number pad hosted by this rule and waits for the app lock to open
 *
 * A timeout reports how many digits the pad still shows and which keys were off screen, never the
 * PIN. The pad clears a rejected PIN, so zero digits can also mean a full PIN was entered and refused
 */
fun ComposeTestRule.enterPinAndAwaitUnlock(pin: String) {
    val retapped = enterPinDigits(pin)

    // the pad checks a full PIN after coroutine delays on the test's virtual clock
    mainClock.advanceTimeBy(PIN_SUBMIT_SETTLE_MILLIS)

    try {
        waitUntil(UNLOCK_TIMEOUT_MILLIS) { !AuthManager.getInstance().isLocked }
    } catch (timeout: ComposeTimeoutException) {
        val hiddenKeys = ('0'..'9').filterNot { onNodeWithTag("numberPadPin.digit.$it").isDisplayed() }
        throw AssertionError(
            "the app stayed locked after the PIN: the pad shows ${shownPinDigits()} of ${pin.length} digits, " +
                "keys not displayed: $hiddenKeys, keys tapped twice: $retapped",
            timeout,
        )
    }

    waitForIdle()
}

/**
 * Types [pin] on a number pad hosted by this rule, one confirmed digit at a time, and returns how
 * many keys needed a second tap
 *
 * Each tap reaches the pad's window as separate injected events, so a relayout between them, such
 * as the lock window settling after it opens or the app returning to the foreground, can cancel a
 * tap before the pad sees it. Typing on regardless leaves the pad one digit short and the app
 * locked, so each key waits for its dot and is tapped once more if the dot never came. It returns
 * without failing, so the caller's own check reports what the pad ended up with
 */
fun ComposeTestRule.enterPinDigits(pin: String): Int {
    var retapped = 0
    pin.forEach { digit ->
        val before = shownPinDigits()
        if (!tapPinDigit(digit, before)) {
            retapped++
            tapPinDigit(digit, before)
        }
    }

    return retapped
}

// a tap the pad took always changes its dot count: one more dot, or none once a full PIN is checked
// and cleared or the lock closes. The pad appends the digit inside the click, and every semantics
// read waits for idle, so a count still unchanged after the wait means the tap never reached the
// pad and a second tap cannot type the digit twice
private fun ComposeTestRule.tapPinDigit(
    digit: Char,
    before: Int,
): Boolean {
    onNodeWithTag("numberPadPin.digit.$digit").performClick()

    return try {
        waitUntil(DIGIT_TIMEOUT_MILLIS) { shownPinDigits() != before || !AuthManager.getInstance().isLocked }
        true
    } catch (_: ComposeTimeoutException) {
        false
    }
}

private fun ComposeTestRule.shownPinDigits(): Int = onAllNodesWithText(PIN_DOT, useUnmergedTree = true).fetchSemanticsNodes().size

private const val PIN_DOT = "\u25CF"
private const val PIN_SUBMIT_SETTLE_MILLIS = 1_000L
private const val UNLOCK_TIMEOUT_MILLIS = 15_000L
private const val DIGIT_TIMEOUT_MILLIS = 5_000L
