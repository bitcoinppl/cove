package org.bitcoinppl.cove.test

import android.os.SystemClock
import android.util.Log
import androidx.test.platform.app.InstrumentationRegistry
import androidx.test.uiautomator.By
import androidx.test.uiautomator.UiDevice
import androidx.test.uiautomator.Until
import org.bitcoinppl.cove.cloudbackup.CloudBackupManager
import org.bitcoinppl.cove_core.CloudBackupRootPrompt

private const val PROMPT_LOG_TAG = "CloudBackupPrompts"
private const val PROMPT_QUIET_MS = 3_000L
private const val PROMPT_SETTLE_TIMEOUT_MS = 30_000L
private const val PROMPT_POLL_MS = 200L
private const val PROMPT_BUTTON_TIMEOUT_MS = 10_000L
private const val PROMPT_DISMISS_TIMEOUT_MS = 10_000L

/**
 * Wave away the cloud backup verification prompt with Not Now, as a person would
 *
 * Each wallet change asks to verify the updated backup, in a dialog over whatever screen is
 * showing except the Cloud Backup detail, after a short delay. It has nothing to do with the screen
 * under test but hides it from UiAutomator, so this waits until no prompt has been pending for
 * [PROMPT_QUIET_MS], dismissing each one that shows
 */
fun UiDevice.dismissCloudBackupVerificationPrompts() {
    val deadline = SystemClock.uptimeMillis() + PROMPT_SETTLE_TIMEOUT_MS
    var quietSince = SystemClock.uptimeMillis()

    while (SystemClock.uptimeMillis() - quietSince < PROMPT_QUIET_MS) {
        check(SystemClock.uptimeMillis() < deadline) {
            "the cloud backup verification prompt kept coming back\n${dumpWindowHierarchy()}"
        }

        if (isVerificationPromptPending()) {
            Log.i(PROMPT_LOG_TAG, "dismissing the cloud backup verification prompt")
            dismissVerificationPrompt()
            quietSince = SystemClock.uptimeMillis()
        }

        Thread.sleep(PROMPT_POLL_MS)
    }
}

private fun UiDevice.dismissVerificationPrompt() {
    // the prompt is pending in rust before the dialog shows, so wait for its button
    val notNow =
        wait(Until.findObject(By.text("Not Now")), PROMPT_BUTTON_TIMEOUT_MS)
            ?: error("the verification prompt was pending but never showed Not Now\n${dumpWindowHierarchy()}")
    notNow.click()

    val deadline = SystemClock.uptimeMillis() + PROMPT_DISMISS_TIMEOUT_MS
    while (isVerificationPromptPending()) {
        check(SystemClock.uptimeMillis() < deadline) {
            "Not Now did not dismiss the verification prompt\n${dumpWindowHierarchy()}"
        }

        Thread.sleep(PROMPT_POLL_MS)
    }
}

private fun isVerificationPromptPending(): Boolean {
    var pending = false
    InstrumentationRegistry.getInstrumentation().runOnMainSync {
        pending = CloudBackupManager.getInstance().screen.rootPrompt is CloudBackupRootPrompt.Verification
    }

    return pending
}
