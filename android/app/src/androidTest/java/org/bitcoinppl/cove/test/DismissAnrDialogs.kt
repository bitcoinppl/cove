package org.bitcoinppl.cove.test

import androidx.test.platform.app.InstrumentationRegistry
import androidx.test.uiautomator.By
import androidx.test.uiautomator.UiDevice
import org.junit.runner.Description
import org.junit.runner.notification.RunListener

/**
 * Dismisses a system "isn't responding" dialog before each test
 *
 * On a slow emulator another app, usually the launcher, can stop responding. Its dialog then sits
 * on top of the app under test and swallows Back, outside taps and clipboard focus for every later
 * test, so `just android-instrumented-ci` passes this listener to the runner
 */
class DismissAnrDialogs : RunListener() {
    override fun testStarted(description: Description) {
        val device = UiDevice.getInstance(InstrumentationRegistry.getInstrumentation())

        // "Wait" keeps the stalled app alive, so it doesn't restart and stall again mid-test
        repeat(MAX_DIALOGS) {
            val wait = device.findObject(By.res("android", "aerr_wait")) ?: return
            wait.click()
            device.waitForIdle()
        }
    }

    private companion object {
        const val MAX_DIALOGS = 3
    }
}
