package org.bitcoinppl.cove.test

import android.accessibilityservice.AccessibilityServiceInfo
import android.app.UiAutomation
import android.os.SystemClock
import android.view.KeyEvent
import androidx.test.platform.app.InstrumentationRegistry
import androidx.test.uiautomator.UiDevice

/**
 * Presses system Back once a window of the app under test holds input focus
 *
 * Back goes to whichever window holds input focus, while Compose test input and UiAutomator clicks
 * reach the app whatever holds it. A system window that takes focus, such as the launcher's
 * "isn't responding" dialog on a slow emulator, then swallows Back while the rest of the test
 * carries on, and the test fails later on a screen that never changed. Waiting for focus first
 * keeps Back on the app, and a timeout names the window that held focus instead
 */
fun UiDevice.pressBackInApp(timeoutMillis: Long = APP_FOCUS_TIMEOUT_MILLIS) {
    awaitAppInputFocus(timeoutMillis)

    // pressBack() reports whether a content-changed event followed, which Compose screens often
    // don't send, while pressKeyCode() reports whether the key itself was injected
    check(pressKeyCode(KeyEvent.KEYCODE_BACK)) { "UiAutomator could not inject system Back" }
}

/** Waits until a window of the app under test holds input focus, failing with the window that does */
fun UiDevice.awaitAppInputFocus(timeoutMillis: Long = APP_FOCUS_TIMEOUT_MILLIS) {
    val automation = interactiveWindowsAutomation()
    val deadline = SystemClock.uptimeMillis() + timeoutMillis

    while (SystemClock.uptimeMillis() < deadline) {
        if (automation.appHoldsInputFocus()) return

        SystemClock.sleep(FOCUS_POLL_MILLIS)
    }

    if (automation.appHoldsInputFocus()) return

    throw AssertionError(
        "expected a window of ${targetPackage()} to hold input focus within $timeoutMillis ms, " +
            "but the system reports ${focusedWindowSummary()}",
    )
}

private fun UiAutomation.appHoldsInputFocus(): Boolean {
    val packageName = targetPackage()

    // a window UiAutomator can't read, such as the "isn't responding" dialog, has no root, so it
    // never counts as the app's
    return windows.any { window -> window.isFocused && window.root?.packageName == packageName }
}

// window listing only covers every interactive window, dialogs and system windows included, with
// this flag set; UiAutomator usually sets it already, so this only makes sure
private fun interactiveWindowsAutomation(): UiAutomation {
    val automation = InstrumentationRegistry.getInstrumentation().uiAutomation
    val info = automation.serviceInfo
    if (info.flags and AccessibilityServiceInfo.FLAG_RETRIEVE_INTERACTIVE_WINDOWS == 0) {
        info.flags = info.flags or AccessibilityServiceInfo.FLAG_RETRIEVE_INTERACTIVE_WINDOWS
        automation.serviceInfo = info
    }

    return automation
}

// the window manager names the focused window even when accessibility can't read it
private fun UiDevice.focusedWindowSummary(): String =
    executeShellCommand("dumpsys window displays")
        .lineSequence()
        .map { it.trim() }
        .filter { it.startsWith("mCurrentFocus=") || it.startsWith("mFocusedApp=") }
        .distinct()
        .joinToString("; ")
        .ifEmpty { "no focused window" }

private fun targetPackage(): String = InstrumentationRegistry.getInstrumentation().targetContext.packageName

private const val APP_FOCUS_TIMEOUT_MILLIS = 10_000L

private const val FOCUS_POLL_MILLIS = 100L
