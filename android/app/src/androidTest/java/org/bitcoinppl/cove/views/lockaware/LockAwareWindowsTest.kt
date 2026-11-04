package org.bitcoinppl.cove.views.lockaware

import androidx.activity.ComponentActivity
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.rememberModalBottomSheetState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.platform.ViewRootForTest
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.test.platform.app.InstrumentationRegistry
import org.bitcoinppl.cove.Auth
import org.bitcoinppl.cove.test.bootstrapRustRuntimeForUiTest
import org.bitcoinppl.cove.test.enterPinAndAwaitUnlock
import org.bitcoinppl.cove.test.restoreNoAppLock
import org.bitcoinppl.cove.ui.theme.CoveTheme
import org.bitcoinppl.cove.views.LockView
import org.bitcoinppl.cove_core.AppLockReason
import org.bitcoinppl.cove_core.AuthManagerAction
import org.bitcoinppl.cove_core.AuthType
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotEquals
import org.junit.Assert.assertTrue
import org.junit.Assume.assumeTrue
import org.junit.Before
import org.junit.Rule
import org.junit.Test

/**
 * Dialogs, bottom sheets, and menus must not be shown while the app is locked, and dialogs and
 * sheets must keep their state through the lock
 *
 * Each opens its own window above the activity, so the lock screen, which draws inside the activity
 * window, cannot cover them. Dialogs and sheets hide their window, which the platform then neither
 * draws nor sends touches to nor lists for accessibility services, the same as `Dialog.hide()`.
 * Menus close outright, and a window first opened while locked waits for unlock. So this checks
 * whether each window exists and has its root view shown
 */
class LockAwareWindowsTest {
    @get:Rule
    val compose = createAndroidComposeRule<ComponentActivity>()

    private val instrumentation = InstrumentationRegistry.getInstrumentation()
    private var shown by mutableStateOf<LockAwareWindow?>(null)
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
        // a skipped test never set a PIN, so it must not remove the one it found
        if (pinSet) restoreNoAppLock()
    }

    @Test
    fun windowsHideWhileLockedAndKeepTheirStateAfterUnlock() {
        compose.setContent {
            CoveTheme(dynamicColor = false) {
                AppLockScope(Auth) {
                    LockView {
                        shown?.let { LockAwareWindowHost(it) }
                    }
                }
            }
        }
        if (Auth.isLocked) compose.enterPinAndAwaitUnlock(TEST_PIN)

        LockAwareWindow.entries.forEach { window ->
            checkAcrossLock(window)
            checkOpenedWhileLocked(window)
        }
    }

    private fun checkAcrossLock(window: LockAwareWindow) {
        // unlocked, the window is shown and usable, so the locked check below is not vacuous
        compose.runOnUiThread { shown = window }
        val before = window.label(taps = 0)
        compose.waitUntil(STATE_TIMEOUT_MILLIS) { windowState(before) == WindowState.SHOWN }
        compose.onNodeWithText(before).performClick()
        val tapped = window.label(taps = if (window.keepsState) 1 else 0)
        compose.waitUntil(STATE_TIMEOUT_MILLIS) { windowState(tapped) == WindowState.SHOWN }

        // a frozen Compose clock leaves no frames for an exit animation, so the window must go in
        // the frame the lock lands in, not when an animation ends
        compose.mainClock.autoAdvance = false
        instrumentation.runOnMainSync { Auth.lock(AppLockReason.REAUTHENTICATION) }
        compose.mainClock.advanceTimeByFrame()
        assertTrue("the app did not lock", Auth.isLocked)
        val locked = windowState(tapped)
        compose.mainClock.autoAdvance = true
        assertNotEquals("${window.title} was shown while locked", WindowState.SHOWN, locked)

        // a dialog or sheet comes back with the state it had, so work pending inside it survives
        compose.enterPinAndAwaitUnlock(TEST_PIN)
        compose.waitUntil(STATE_TIMEOUT_MILLIS) { windowState(tapped) == WindowState.SHOWN }

        compose.runOnUiThread { shown = null }
        compose.waitUntil(STATE_TIMEOUT_MILLIS) { windowState(tapped) == WindowState.ABSENT }
    }

    private fun checkOpenedWhileLocked(window: LockAwareWindow) {
        // a window opened while locked must wait for unlock, since Android would draw its first frame
        // before it could hide
        instrumentation.runOnMainSync { Auth.lock(AppLockReason.REAUTHENTICATION) }
        assertTrue("the app did not lock", Auth.isLocked)
        compose.runOnUiThread { shown = window }
        compose.waitForIdle()
        val label = window.label(taps = 0)
        assertEquals("${window.title} opened while locked", WindowState.ABSENT, windowState(label))

        compose.enterPinAndAwaitUnlock(TEST_PIN)
        compose.waitUntil(STATE_TIMEOUT_MILLIS) { windowState(label) == WindowState.SHOWN }

        compose.runOnUiThread { shown = null }
        compose.waitUntil(STATE_TIMEOUT_MILLIS) { windowState(label) == WindowState.ABSENT }
    }

    // whether the window showing [label] exists and has its root view shown
    private fun windowState(label: String): WindowState {
        val node =
            compose
                .onAllNodes(hasText(label), useUnmergedTree = true)
                .fetchSemanticsNodes()
                .firstOrNull() ?: return WindowState.ABSENT
        val view = checkNotNull((node.root as? ViewRootForTest)?.view) { "no view for $label" }

        var isShown = false
        instrumentation.runOnMainSync { isShown = view.isShown }
        return if (isShown) WindowState.SHOWN else WindowState.HIDDEN
    }

    private enum class WindowState {
        ABSENT,
        HIDDEN,
        SHOWN,
    }

    private companion object {
        const val TEST_PIN = "864213"
        const val STATE_TIMEOUT_MILLIS = 15_000L
    }
}

/** A lock-aware window kind, shown one at a time */
private enum class LockAwareWindow(
    val title: String,
    // a menu closes on lock instead of hiding, so its tap count would start over
    val keepsState: Boolean,
) {
    ALERT_DIALOG("Alert", keepsState = true),
    DIALOG("Dialog", keepsState = true),
    BOTTOM_SHEET("Sheet", keepsState = true),
    DROPDOWN_MENU("Menu", keepsState = false),
    ;

    fun label(taps: Int): String = "$title taps: $taps"
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun LockAwareWindowHost(window: LockAwareWindow) {
    when (window) {
        LockAwareWindow.ALERT_DIALOG ->
            AlertDialog(
                onDismissRequest = {},
                confirmButton = { TapCounter(window) },
            )

        LockAwareWindow.DIALOG ->
            Dialog(onDismissRequest = {}) { TapCounter(window) }

        LockAwareWindow.BOTTOM_SHEET ->
            ModalBottomSheet(
                onDismissRequest = {},
                sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true),
            ) {
                TapCounter(window)
            }

        LockAwareWindow.DROPDOWN_MENU ->
            DropdownMenu(expanded = true, onDismissRequest = {}) {
                DropdownMenuItem(text = { Text(window.label(taps = 0)) }, onClick = {})
            }
    }
}

// state remembered inside the window, like a picker launcher or a half-finished form would be
@Composable
private fun TapCounter(window: LockAwareWindow) {
    var taps by remember { mutableIntStateOf(0) }
    TextButton(onClick = { taps++ }) { Text(window.label(taps)) }
}
