package org.bitcoinppl.cove.flows.cloudbackup

import android.content.Intent
import android.graphics.Rect
import android.os.Bundle
import android.provider.Settings
import android.system.Os
import android.util.Log
import android.view.accessibility.AccessibilityNodeInfo
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import androidx.test.uiautomator.By
import androidx.test.uiautomator.BySelector
import androidx.test.uiautomator.Configurator
import androidx.test.uiautomator.StaleObjectException
import androidx.test.uiautomator.UiDevice
import androidx.test.uiautomator.UiObject2
import androidx.test.uiautomator.Until
import org.bitcoinppl.cove.test.AndroidDeviceStayAwakeRule
import org.bitcoinppl.cove.test.RealCloudAccountTest
import org.bitcoinppl.cove.test.dumpWindowHierarchy
import org.bitcoinppl.cove.test.launchFullApp
import org.junit.Assert.assertTrue
import org.junit.Assume.assumeTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import java.util.regex.Pattern

/**
 * Restores the frozen backup of a real Google account through real Google Drive and Google
 * Password Manager, then checks that every wallet and the marker labels came back
 *
 * Runs only through `just android-cloud-backup-e2e`, which factory-resets a Play Store emulator and
 * puts the account in a file on it; docs/testing.md describes the account
 *
 * The app reads the real Drive but never writes to it, since a finished restore uploads every
 * wallet again and would otherwise replace the frozen backup with what the code under test wrote
 */
@RealCloudAccountTest
@RunWith(AndroidJUnit4::class)
class RealCloudBackupRestoreTest {
    @get:Rule
    val stayAwake = AndroidDeviceStayAwakeRule()

    private val device = UiDevice.getInstance(InstrumentationRegistry.getInstrumentation())

    @Test
    fun restoresFrozenBackupWithWalletsAndLabels() {
        val account = RealCloudAccount.fromDeviceOrSkip(device)

        GoogleSignInRobot(device, account).ensureSignedIn()

        // Google's device backup offer opens over whatever is on screen a minute or so after a
        // sign-in, so the app that shows it is turned off before the restore
        device.executeShellCommand("pm disable-user --user 0 $GOOGLE_RESTORE_PACKAGE")

        // the test runs in the app's process, so the app sees this before its first Drive request
        Os.setenv(DRIVE_READ_ONLY_ENV, "1", true)
        launchFullApp(resetData = true)
        CloudRestoreRobot(device, account).restoreFromOnboarding()

        val app = RestoredAppRobot(device)
        FROZEN_BACKUP.groupBy { it.network }.forEach { (network, wallets) ->
            app.switchNetwork(network)
            app.assertWalletsListed(wallets.map { it.name })

            wallets.mapNotNull { wallet -> wallet.markerLabel?.let { wallet.name to it } }.forEach { (name, label) ->
                app.assertTransactionLabel(name, label)
            }
        }

        app.assertCloudBackupInventory(FROZEN_BACKUP)
    }
}

/**
 * A wallet in the frozen backup, as the Cloud Backup screen describes it
 *
 * Changing the backup means changing this list; docs/testing.md says how
 */
private data class FrozenWallet(
    val name: String,
    val network: String,
    /** The row's detail line, such as `Signet • Hot • D317F698` */
    val detail: String,
    val labelCount: Int,
    /** A transaction label added for this test, which the wallet's transaction list must show */
    val markerLabel: String? = null,
)

private val FROZEN_BACKUP =
    listOf(
        FrozenWallet("abandon", "Bitcoin", "Bitcoin • Hot • 73C5DA0A", 9, markerLabel = "E2E Bitcoin label"),
        FrozenWallet("main", "Signet", "Signet • Hot • 560072A4", 0),
        FrozenWallet("SignerLab", "Signet", "Signet • Cold • 1A0B9334", 0),
        FrozenWallet("Imported 07AD5F46", "Signet", "Signet • Cold • 07AD5F46", 0),
        FrozenWallet("Wallet 4", "Signet", "Signet • Hot • 73C5DA0A", 0),
        FrozenWallet("secondary", "Signet", "Signet • Hot • D317F698", 6, markerLabel = "E2E Signet label"),
        FrozenWallet("Imported XPub", "Signet", "Signet • Xpub Only", 0),
        FrozenWallet("Wallet 1", "Testnet", "Testnet • Hot • FA498BE1", 0),
    )

/** The Google account holding the frozen backup, from the file the runner puts on the emulator */
private class RealCloudAccount(
    val email: String,
    val password: String,
    /** The emulator's screen lock, which is also the lock of the test emulator before it */
    val screenLockPin: String,
) {
    // keeps the password out of failure messages
    override fun toString(): String = "RealCloudAccount"

    companion object {
        // written by the android-cloud-backup-e2e runner through adb's stdin, readable only by the
        // shell user; instrumentation arguments would leave the password in logs under build/
        private const val ACCOUNT_FILE = "/data/local/tmp/cove-real-cloud-account"

        fun fromDeviceOrSkip(device: UiDevice): RealCloudAccount {
            val lines = device.executeShellCommand("cat $ACCOUNT_FILE").lines()
            device.executeShellCommand("rm -f $ACCOUNT_FILE")

            val (email, password, pin) = lines.take(3) + List(3) { "" }
            assumeTrue(
                "needs the real Google account; run it with just android-cloud-backup-e2e",
                email.isNotEmpty() && password.isNotEmpty() && pin.isNotEmpty(),
            )

            return RealCloudAccount(email, password, pin)
        }
    }
}

/** Adds the Google account through Settings, answering each setup screen Google shows */
private class GoogleSignInRobot(
    private val device: UiDevice,
    private val account: RealCloudAccount,
) {
    fun ensureSignedIn() {
        if (isSignedIn()) return

        openAddGoogleAccount()

        val deadline = System.currentTimeMillis() + SIGN_IN_TIMEOUT_MILLIS
        var closedSince: Long? = null

        while (System.currentTimeMillis() < deadline) {
            failOnChallenge()

            // the account appears before Google's last setup screens close
            if (isSignedIn() && device.currentPackageName != GMS_PACKAGE) return

            // a fresh Play Store image updates Play services soon after boot, which ends the
            // sign-in it was showing, so the flow starts again once none of its windows is left
            val now = System.currentTimeMillis()
            val flowShowing = device.hasObject(By.pkg(GMS_PACKAGE)) || device.hasObject(By.pkg(SETTINGS_PACKAGE))
            closedSince = if (flowShowing) null else closedSince ?: now

            if (closedSince != null && now - closedSince >= FLOW_CLOSED_MILLIS) {
                Log.i(TAG, "Google sign-in: the flow closed before the account was added, opening it again")
                openAddGoogleAccount()
                closedSince = null
            }

            if (!answerVisibleScreen()) Thread.sleep(500)
        }

        error("Google sign-in did not finish\n${device.dumpWindowHierarchy()}")
    }

    private fun isSignedIn(): Boolean =
        device
            .executeShellCommand("dumpsys account")
            .contains("Account {name=${account.email}, type=com.google}")

    private fun openAddGoogleAccount() {
        val instrumentation = InstrumentationRegistry.getInstrumentation()
        val intent =
            Intent(Settings.ACTION_ADD_ACCOUNT)
                .putExtra(Settings.EXTRA_ACCOUNT_TYPES, arrayOf("com.google"))
                .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_CLEAR_TASK)

        instrumentation.context.startActivity(intent)
    }

    private fun failOnChallenge() {
        val challenge =
            listOf("Verify it’s you", "Verify it's you", "Couldn’t sign you in", "Couldn't sign you in", "Wrong password")
                .firstOrNull { device.hasObject(By.textContains(it)) }
                ?: return

        error(
            "Google stopped the sign-in with \"$challenge\"; sign in once by hand on the emulator, " +
                "then rerun with --keep-data\n${device.dumpWindowHierarchy()}",
        )
    }

    /** Answers the setup screen on top, returning false when none it knows is showing */
    private fun answerVisibleScreen(): Boolean {
        // checked before the phone-number screen, since this one has a Skip button too
        if (device.hasObject(By.textStartsWith("Enter the screen lock for your other device"))) {
            Log.i(TAG, "Google sign-in: unlocking passkeys with another device's screen lock")
            unlockPasskeysWithOtherDevice()
            return true
        }

        if (device.hasObject(By.text("Sign in with ease"))) {
            Log.i(TAG, "Google sign-in: skipping phone number sign-in")
            return device.clickGoogleButton("Skip")
        }

        device.findObject(By.res("identifierId"))?.let { emailField ->
            Log.i(TAG, "Google sign-in: entering the email")
            submitField(emailField, account.email, By.res("identifierId"))
            return true
        }

        if (device.hasObject(By.text("Show password"))) {
            Log.i(TAG, "Google sign-in: entering the password")
            val passwordField = device.findObject(By.clazz("android.widget.EditText")) ?: return false
            submitField(passwordField, account.password, By.text("Show password"))
            return true
        }

        val answered =
            device.clickGoogleButton("I agree") ||
                // recovery options
                (device.hasObject(By.text("Make sure you can always sign in")) && device.clickGoogleButton("Cancel")) ||
                (device.hasObject(By.text("Set a home address")) && device.clickGoogleButton("Skip")) ||
                (device.hasObject(By.text("Add a selfie for sign-in")) && device.clickGoogleButton("Not now")) ||
                // device backup, which would add this emulator's data to the account
                device.clickIfPresent(By.text(Pattern.compile("Don.t back up")))

        if (answered) Log.i(TAG, "Google sign-in: answered a setup screen in ${device.currentPackageName}")
        return answered
    }

    /** Fills a sign-in field and submits it, then waits for Google to move past its page */
    private fun submitField(
        field: UiObject2,
        value: String,
        pageMarker: BySelector,
    ) {
        device.setTextQuietly(field, value)

        // the page's button is a web element, so Enter submits the form when it can't be found
        if (!device.clickGoogleButton("Next")) {
            Log.i(TAG, "Google sign-in: no Next button, pressing Enter")
            device.pressEnter()
        }

        val moved = device.wait(Until.gone(pageMarker), 20_000) == true
        Log.i(TAG, "Google sign-in: ${if (moved) "page submitted" else "page still showing"}")
    }

    /**
     * Google Password Manager keeps passkeys end-to-end encrypted, so a new device needs the screen
     * lock of one that has them; the newest Android device on the list is the previous test
     * emulator, which had the same PIN as this one
     */
    private fun unlockPasskeysWithOtherDevice() {
        device.clickGoogleButton("Select a device")

        val otherDevice =
            device.wait(Until.findObject(By.text(Pattern.compile("^Android\\b.*"))), 10_000)
                ?: error("No earlier Android test emulator to unlock passkeys with\n${device.dumpWindowHierarchy()}")
        otherDevice.click()
        device.waitForIdle()

        enterOtherDeviceScreenLock()
        if (!device.clickGoogleButton("Submit")) device.pressEnter()

        // Google Password Manager rejects a wrong lock on the same screen, and only a few tries
        // are allowed before it locks the account's passkeys
        device.wait(Until.gone(By.textStartsWith("Enter the screen lock for your other device")), 60_000)
        if (device.hasObject(By.textStartsWith("Enter the screen lock for your other device"))) {
            error("Google Password Manager did not accept the screen lock\n${device.dumpWindowHierarchy()}")
        }
    }

    private fun enterOtherDeviceScreenLock() {
        repeat(SCREEN_LOCK_FIELD_ATTEMPTS) {
            val pinField =
                device.wait(Until.findObject(By.clazz("android.widget.EditText")), 10_000)
                    ?: error("No screen lock field\n${device.dumpWindowHierarchy()}")

            // the page ignores text set on this field through accessibility, so the PIN is typed as
            // key events once the field has focus, as the restore's screen lock prompt is
            try {
                pinField.click()
                device.waitForIdle()
                device.executeShellCommand("input text ${account.screenLockPin}")
                return
            } catch (_: StaleObjectException) {
                // the page redraws its field while it finishes loading, so the field is found again
                Log.i(TAG, "Google sign-in: the screen lock field changed, finding it again")
                device.waitForIdle()
            }
        }

        error("The screen lock field kept changing\n${device.dumpWindowHierarchy()}")
    }

    private companion object {
        const val SIGN_IN_TIMEOUT_MILLIS = 5 * 60_000L
        const val SCREEN_LOCK_FIELD_ATTEMPTS = 5
        const val FLOW_CLOSED_MILLIS = 10_000L
    }
}

/** Restores from the onboarding offer through Drive authorization and the passkey */
private class CloudRestoreRobot(
    private val device: UiDevice,
    private val account: RealCloudAccount,
) {
    fun restoreFromOnboarding() {
        reachRestoreOffer()
        device.clickOrFail(By.text("Restore with Passkey"))
        answerSystemPromptsUntilRestored()
        device.waitOrFail(By.text("Your wallets have been restored."))

        // labels that failed to import are reported on the same finished screen
        failOnRestoreProblem()
        device.clickOrFail(By.text("Done"))

        // the restore ends on the terms, which are accepted again, once they replace the Done screen
        device.waitOrFail(By.res(Pattern.compile("onboarding\\.terms\\.check\\.backup|selectedWallet\\.title")), 30_000)
        acceptTermsIfShown()
        device.waitOrFail(By.res("selectedWallet.title"), timeoutMillis = 60_000)
    }

    /** Fresh installs start on the terms, then Welcome, unless the silent Drive check finds the backup first */
    private fun reachRestoreOffer() {
        val deadline = System.currentTimeMillis() + 90_000

        while (System.currentTimeMillis() < deadline) {
            if (device.hasObject(By.text("Restore with Passkey"))) return

            acceptTermsIfShown() ||
                device.clickIfPresent(By.res("onboarding.getStarted")) ||
                device.clickIfPresent(By.res("onboarding.bitcoinChoice.restore"))
            Thread.sleep(500)
        }

        error("The onboarding never offered the cloud restore\n${device.dumpWindowHierarchy()}")
    }

    private fun acceptTermsIfShown(): Boolean {
        if (!device.hasObject(By.res("onboarding.terms.check.backup"))) return false

        listOf("backup", "legal", "financial", "recovery", "agreement").forEach { check ->
            val box = device.waitOrFail(By.res("onboarding.terms.check.$check"))

            if (!box.isChecked) box.click()
        }

        device.clickOrFail(By.res("onboarding.terms.agree"))
        device.waitForIdle()
        return true
    }

    /**
     * Cove shows its restoring screen at once, and Drive authorization may then put Google's
     * account chooser and consent over it, then Credential Manager shows the passkey and asks for
     * the screen lock; which of them appear depends on what this device and account granted before
     */
    private fun answerSystemPromptsUntilRestored() {
        val deadline = System.currentTimeMillis() + RESTORE_TIMEOUT_MILLIS

        while (System.currentTimeMillis() < deadline) {
            if (device.hasObject(By.text("You're all set"))) return

            failOnRestoreProblem()
            if (!answerSystemPrompt()) Thread.sleep(500)
        }

        error("The restore did not finish\n${device.dumpWindowHierarchy()}")
    }

    private fun answerSystemPrompt(): Boolean {
        if (device.currentPackageName == APP_PACKAGE) return false

        if (device.hasObject(By.text("Use your screen lock"))) {
            Log.i(TAG, "Restore: entering the screen lock for the passkey")
            device.executeShellCommand("input text ${account.screenLockPin}")
            device.executeShellCommand("input keyevent KEYCODE_ENTER")
            device.wait(Until.gone(By.text("Use your screen lock")), 10_000)
            return true
        }

        val answered =
            device.clickIfPresent(By.text(account.email)) ||
                device.clickIfPresent(By.text("Allow")) ||
                device.clickIfPresent(By.text("Continue"))

        if (answered) Log.i(TAG, "Restore: answered a prompt in ${device.currentPackageName}")
        return answered
    }

    private fun failOnRestoreProblem() {
        val problem =
            listOf("Restore Failed", "Some wallets were restored", "could not be imported", "No passkeys available")
                .firstOrNull { device.hasObject(By.textContains(it)) }
                ?: return

        error("The restore reported \"$problem\"\n${device.dumpWindowHierarchy()}")
    }

    private companion object {
        const val RESTORE_TIMEOUT_MILLIS = 8 * 60_000L
    }
}

/** Checks the restored app against the frozen backup */
private class RestoredAppRobot(
    private val device: UiDevice,
) {
    private var network = "Bitcoin"

    fun switchNetwork(target: String) {
        if (network == target) return

        openSettings()
        device.clickOrFail(By.text("Network"))
        device.clickOrFail(By.text(target))
        device.clickOrFail(By.text("Yes, Change Network"))
        waitForWalletDismissingNodeFailure()
        network = target
    }

    /**
     * A test network's default public node is sometimes down, which has nothing to do with the
     * restore, and the app still lists the network's wallets once its alert is dismissed
     */
    private fun waitForWalletDismissingNodeFailure() {
        val deadline = System.currentTimeMillis() + 60_000

        while (System.currentTimeMillis() < deadline) {
            if (device.hasObject(By.res("selectedWallet.title"))) return

            if (device.hasObject(By.text("Node Connection Failed"))) {
                Log.i(TAG, "Network: dismissing a node connection failure")
                device.clickOrFail(By.text("Cancel"))
            }

            Thread.sleep(500)
        }

        error("The wallet screen never came back after changing network\n${device.dumpWindowHierarchy()}")
    }

    fun assertWalletsListed(names: List<String>) {
        openSidebar()
        names.forEach { name -> device.waitOrFail(By.text(name)) }
        closeSidebar()
    }

    /** History comes from the network's real Esplora or Electrum server, so this waits for it */
    fun assertTransactionLabel(
        wallet: String,
        label: String,
    ) {
        // an open sidebar leaves the selected wallet's title on screen beside its row, so the
        // sidebar is used only to change wallets, when the title can't match the row's text
        if (!device.hasObject(selectedWalletTitle(wallet))) {
            openSidebar()
            device.clickOrFail(By.text(wallet))
        }

        device.waitOrFail(selectedWalletTitle(wallet), timeoutMillis = 30_000)
        if (waitForHistory(wallet, label)) return

        // other people keep paying to these public test wallets, so the label may be further down
        var previous: List<String>? = null
        repeat(MAX_HISTORY_SCROLLS) {
            if (device.hasObject(By.text(label))) return

            val screen = screenTexts()
            if (screen == previous) {
                error("$wallet's history ends without the transaction label \"$label\"\n${device.dumpWindowHierarchy()}")
            }

            previous = screen
            device.scrollDown()
        }

        error("$wallet showed no transaction label \"$label\" in its first $MAX_HISTORY_SCROLLS screens\n${device.dumpWindowHierarchy()}")
    }

    /**
     * Watches the top of the history until [label] shows there, returning true, or the history
     * stops changing, returning false
     *
     * Rows show while the wallet's first scan runs and the newest can come last, after the scan's
     * progress text has gone, so the list is read further down only once it has held still
     */
    private fun waitForHistory(
        wallet: String,
        label: String,
    ): Boolean {
        val deadline = System.currentTimeMillis() + HISTORY_TIMEOUT_MILLIS
        var screen = screenTexts()
        var stillSince = System.currentTimeMillis()

        while (System.currentTimeMillis() < deadline) {
            if (device.hasObject(By.text(label))) return true

            val scanning =
                device.hasObject(By.textStartsWith("Scanning for transactions")) ||
                    device.hasObject(By.text("Checking wallet history"))
            val settled = System.currentTimeMillis() - stillSince >= HISTORY_SETTLE_MILLIS

            if (!scanning && settled && device.hasObject(TRANSACTION_DATE)) {
                Log.i(TAG, "History: $wallet settled without \"$label\" on top: ${screen.take(12)}")
                return false
            }

            Thread.sleep(1_000)

            val next = screenTexts()
            if (next != screen) {
                screen = next
                stillSince = System.currentTimeMillis()
            }
        }

        error("$wallet's history never settled\n${device.dumpWindowHierarchy()}")
    }

    /** Every text on screen with where it is, which only stays the same when nothing moved */
    private fun screenTexts(): List<String> =
        device.findObjects(By.clazz("android.widget.TextView")).mapNotNull { node ->
            try {
                "${node.text}@${node.visibleCenter}"
            } catch (_: StaleObjectException) {
                null
            }
        }

    fun assertCloudBackupInventory(expected: List<FrozenWallet>) {
        openSettings()
        // Cloud Backup is both the section's header and its row, and only the row opens the screen
        device.clickOrFail(By.clickable(true).hasDescendant(By.text("Cloud Backup")), scroll = true)
        device.waitOrFail(By.text("Cloud Backup Active"), timeoutMillis = 60_000)

        // the list is longer than the screen, so rows are collected while it scrolls
        val rows = mutableMapOf<String, List<String>>()
        for (attempt in 0 until 10) {
            device.findObjects(By.res(Pattern.compile("cloudBackup\\.wallet\\..*"))).forEach { row ->
                val texts = rowTexts(row) ?: return@forEach
                texts.firstOrNull()?.let { name -> rows[name] = texts }
            }

            if (expected.all { it.name in rows }) break
            device.scrollDown()
        }

        expected.forEach { wallet ->
            val texts = rows[wallet.name] ?: error("Cloud Backup does not list ${wallet.name}; it lists ${rows.keys}")
            val labels = "${wallet.labelCount} ${if (wallet.labelCount == 1) "label" else "labels"}"

            assertTrue("${wallet.name} shows $texts, not ${wallet.detail}", wallet.detail in texts)
            assertTrue("${wallet.name} shows $texts, not $labels", labels in texts)
        }
    }

    /** The row's texts in order, or null when it scrolled away while being read */
    private fun rowTexts(row: UiObject2): List<String>? =
        try {
            row.findObjects(By.clazz("android.widget.TextView")).mapNotNull { it.text }
        } catch (_: StaleObjectException) {
            null
        }

    private fun openSidebar() {
        device.clickOrFail(By.desc("Menu"))
        device.waitOrFail(By.text("My Wallets"))
    }

    /** Back on the root screen with the sidebar open leaves the app, so the dimmed backdrop is tapped */
    private fun closeSidebar() {
        device.click(device.displayWidth * 19 / 20, device.displayHeight / 2)

        if (!device.wait(Until.gone(By.text("My Wallets")), 10_000)) {
            error("The sidebar did not close\n${device.dumpWindowHierarchy()}")
        }
    }

    private fun openSettings() {
        openSidebar()
        device.clickOrFail(By.text("Settings"))
        device.waitOrFail(By.text("Network"))
    }

    private companion object {
        const val HISTORY_TIMEOUT_MILLIS = 3 * 60_000L
        const val MAX_HISTORY_SCROLLS = 15
        const val HISTORY_SETTLE_MILLIS = 10_000L

        // a confirmed transaction row's date, such as August 21, 2026
        val TRANSACTION_DATE: BySelector = By.text(Pattern.compile("^[A-Z][a-z]+ \\d{1,2}, \\d{4}$"))
    }
}

// the title's text is on a child of the tagged node
private fun selectedWalletTitle(wallet: String): BySelector = By.res("selectedWallet.title").hasDescendant(By.text(wallet))

private const val TAG = "RealCloudBackupRestore"
private const val APP_PACKAGE = "org.bitcoinppl.cove.dev"
private const val GMS_PACKAGE = "com.google.android.gms"
private const val SETTINGS_PACKAGE = "com.android.settings"
private const val GOOGLE_RESTORE_PACKAGE = "com.google.android.apps.restore"

// UiTestSeams in the debug source set reads it
private const val DRIVE_READ_ONLY_ENV = "COVE_UI_TEST_DRIVE_READ_ONLY"

private fun UiDevice.waitOrFail(
    selector: BySelector,
    timeoutMillis: Long = 20_000,
): UiObject2 =
    wait(Until.findObject(selector), timeoutMillis)
        ?: error("Timed out waiting for $selector\n${dumpWindowHierarchy()}")

private fun UiDevice.clickOrFail(
    selector: BySelector,
    scroll: Boolean = false,
) {
    if (scroll) {
        repeat(6) {
            if (hasObject(selector)) return@repeat
            scrollDown()
        }
    }

    waitOrFail(selector)

    // the sidebar and screen transitions slide, and recomposition replaces nodes while they do, so
    // the tap goes to where the node came to rest rather than through a node that may be stale
    val bounds = settledBounds(selector) ?: error("$selector never stopped moving\n${dumpWindowHierarchy()}")
    click(bounds.centerX(), bounds.centerY())
    waitForIdle()
}

private fun UiDevice.settledBounds(selector: BySelector): Rect? {
    val deadline = System.currentTimeMillis() + 20_000
    var previous: Rect? = null

    while (System.currentTimeMillis() < deadline) {
        val bounds =
            try {
                findObject(selector)?.visibleBounds
            } catch (_: StaleObjectException) {
                null
            }

        if (bounds != null && bounds == previous) return bounds

        previous = bounds
        Thread.sleep(150)
    }

    return null
}

private fun UiDevice.clickIfPresent(selector: BySelector): Boolean {
    val target = findObject(selector) ?: return false

    return try {
        target.click()
        waitForIdle()
        true
    } catch (_: StaleObjectException) {
        false
    }
}

/**
 * Types [value] into [field] through the accessibility node itself, since UiObject2's text setter
 * logs what it types, which would put the account's password in logcat and Gradle's report
 */
private fun UiDevice.setTextQuietly(
    field: UiObject2,
    value: String,
) {
    field.click()
    waitForIdle()

    val uiAutomation = InstrumentationRegistry.getInstrumentation().getUiAutomation(Configurator.getInstance().uiAutomationFlags)
    val focused =
        uiAutomation.findFocus(AccessibilityNodeInfo.FOCUS_INPUT)
            ?: error("The sign-in field took no input focus\n${dumpWindowHierarchy()}")
    val arguments = Bundle().apply { putCharSequence(AccessibilityNodeInfo.ACTION_ARGUMENT_SET_TEXT_CHARSEQUENCE, value) }

    check(focused.performAction(AccessibilityNodeInfo.ACTION_SET_TEXT, arguments)) {
        "The sign-in field did not take its text\n${dumpWindowHierarchy()}"
    }
}

/** Clicks a button on a Google web page, whose label may be styled in capitals or only described */
private fun UiDevice.clickGoogleButton(label: String): Boolean {
    val pattern = Pattern.compile("^\\s*${Pattern.quote(label)}\\s*$", Pattern.CASE_INSENSITIVE)

    return clickIfPresent(By.text(pattern)) || clickIfPresent(By.desc(pattern))
}

private fun UiDevice.scrollDown() {
    swipe(displayWidth / 2, displayHeight * 3 / 4, displayWidth / 2, displayHeight / 3, 20)
    waitForIdle()
}
