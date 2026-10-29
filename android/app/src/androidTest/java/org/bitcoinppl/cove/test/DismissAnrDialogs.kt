package org.bitcoinppl.cove.test

import android.util.Log
import androidx.test.platform.app.InstrumentationRegistry
import androidx.test.uiautomator.By
import androidx.test.uiautomator.StaleObjectException
import androidx.test.uiautomator.UiDevice
import org.junit.runner.Description
import org.junit.runner.notification.RunListener
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit
import kotlin.concurrent.thread

/**
 * Dismisses system "isn't responding" dialogs before and during each test
 *
 * On a slow emulator the system server or the launcher can stop responding. Its dialog then sits on
 * top of the app under test and swallows Back, taps and clipboard focus, so `just
 * android-instrumented-ci` passes this listener to the runner. "Wait" only holds the dialog off
 * until the next stall, so a watcher keeps checking while the test runs; that covers every robot's
 * wait loop and system Back press without each helper having to know about the dialog
 */
class DismissAnrDialogs : RunListener() {
    private var watcher: AnrDialogWatcher? = null

    override fun testStarted(description: Description) {
        val device = UiDevice.getInstance(InstrumentationRegistry.getInstrumentation())

        // clear a dialog left over from the previous test before this one touches the screen
        device.dismissAnrDialogs()
        watcher = AnrDialogWatcher(device).also { it.start() }
    }

    override fun testFinished(description: Description) {
        watcher?.stop()
        watcher = null
    }
}

private class AnrDialogWatcher(
    private val device: UiDevice,
) {
    private val stopped = CountDownLatch(1)
    private val worker =
        thread(start = false, isDaemon = true, name = "DismissAnrDialogs") {
            while (!stopped.await(POLL_INTERVAL_MILLIS, TimeUnit.MILLISECONDS)) {
                // an uncaught exception on this thread would crash the whole run, not just one
                // test, so log it and try again on the next poll
                try {
                    device.dismissAnrDialogs()
                } catch (e: RuntimeException) {
                    Log.w(TAG, "ANR dialog check failed", e)
                }
            }
        }

    fun start() {
        worker.start()
    }

    fun stop() {
        stopped.countDown()
        worker.join()
    }
}

private fun UiDevice.dismissAnrDialogs() {
    // "Wait" keeps the stalled process alive, so it doesn't restart and stall again mid-test, and
    // never touches the app under test the way "Close app" could
    repeat(MAX_DIALOGS) {
        val wait = findObject(ANR_WAIT_BUTTON) ?: return

        try {
            wait.click()
        } catch (_: StaleObjectException) {
            // the dialog closed between finding and clicking it
            return
        }

        waitForIdle()
    }
}

private val ANR_WAIT_BUTTON = By.pkg("android").res("android", "aerr_wait")

private const val MAX_DIALOGS = 3

private const val TAG = "DismissAnrDialogs"

// the full-launch waits give the app 20 seconds, so a dialog never covers it for long
private const val POLL_INTERVAL_MILLIS = 1_000L
