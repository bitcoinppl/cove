package org.bitcoinppl.cove.test

import android.os.ParcelFileDescriptor
import android.view.WindowManager
import androidx.test.runner.lifecycle.ActivityLifecycleMonitorRegistry
import androidx.test.runner.lifecycle.Stage
import androidx.test.platform.app.InstrumentationRegistry
import androidx.test.uiautomator.By
import androidx.test.uiautomator.BySelector
import androidx.test.uiautomator.UiDevice
import androidx.test.uiautomator.UiObject2
import androidx.test.uiautomator.Until
import java.io.ByteArrayOutputStream
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Assert.fail

class FullLaunchStartupRobot(
    private val device: UiDevice,
) {
    fun assertBootstrappedIntoOnboarding(): FullLaunchStartupRobot {
        assertNull(device.findObject(By.textContains("App startup timed out")))
        assertNull(device.findObject(By.textContains("App initialization error")))

        val termsSelector = tag("onboarding.terms.check.backup")

        if (device.waitUntilAnyVisible(termsSelector, tag("onboarding.getStarted")).description == termsSelector.description) {
            acceptTermsAndContinue()
        }

        device.advanceUntilVisible(tag("onboarding.getStarted"))

        return this
    }

    fun assertScreenshotsAllowed(): FullLaunchStartupRobot {
        assertFlagSecureSet(false, "expected screenshots to be allowed")

        return this
    }

    fun acceptTermsAndContinue(): FullLaunchStartupRobot {
        device.waitUntilVisible(text("Terms & Conditions"))
        device.acceptTerms()
        device.waitUntilVisible(tag("onboarding.getStarted"))

        return this
    }
}

class FullLaunchOnboardingRobot(
    private val device: UiDevice,
) {
    private val knownEmptyMainnetMnemonic =
        listOf(
            "abandon",
            "abandon",
            "abandon",
            "abandon",
            "abandon",
            "abandon",
            "abandon",
            "abandon",
            "abandon",
            "abandon",
            "abandon",
            "about",
    )

    fun tapGetStarted(): FullLaunchOnboardingRobot {
        device.advanceUntilVisible(text("Do you already have Bitcoin?")) {
            clickCenterIfVisible(tag("onboarding.getStarted"))
        }

        return this
    }

    fun chooseExistingUser(): FullLaunchOnboardingRobot {
        device.advanceUntilVisible(text("How do you store your Bitcoin?")) {
            clickCenterIfVisible(tag("onboarding.bitcoinChoice.existing"))
        }

        return this
    }

    fun chooseNewUser(): FullLaunchOnboardingRobot {
        device.advanceUntilVisible(text("Creating your wallet")) {
            clickCenterIfVisible(tag("onboarding.bitcoinChoice.new"))
        }
        device.waitUntilVisible(textContains("Back up your wallet"), timeoutMillis = 10_000)

        return this
    }

    fun goBackToBitcoinChoice(): FullLaunchOnboardingRobot {
        device.waitUntilVisible(text("Back")).click()
        device.advanceUntilVisible(text("Do you already have Bitcoin?"))
        device.waitUntilVisible(tag("onboarding.bitcoinChoice.new"))
        device.waitUntilVisible(tag("onboarding.bitcoinChoice.existing"))

        return this
    }

    fun systemBackToBitcoinChoice(): FullLaunchOnboardingRobot {
        device.pressBack()
        device.advanceUntilVisible(text("Do you already have Bitcoin?"))
        device.waitUntilVisible(tag("onboarding.bitcoinChoice.new"))
        device.waitUntilVisible(tag("onboarding.bitcoinChoice.existing"))

        return this
    }

    fun useAnotherWallet(): FullLaunchOnboardingRobot {
        device.advanceUntilVisible(text("How do you store your Bitcoin?"))

        return this
    }

    fun assertStorageChoices(): FullLaunchOnboardingRobot {
        device.advanceUntilVisible(text("How do you store your Bitcoin?"))
        device.waitUntilVisible(tag("onboarding.storage.exchange"))
        device.waitUntilVisible(tag("onboarding.storage.hardware"))
        device.waitUntilVisible(tag("onboarding.storage.software"))

        return this
    }

    fun chooseExchange(): FullLaunchOnboardingRobot {
        device.advanceUntilVisible(text("Creating your wallet")) {
            clickCenterIfVisible(tag("onboarding.storage.exchange"))
        }
        device.waitUntilVisible(text("Back up your wallet before funding it"), timeoutMillis = 10_000)

        return this
    }

    fun chooseHardwareWallet(): FullLaunchOnboardingRobot {
        device.advanceUntilVisible(text("Import your hardware wallet")) {
            clickCenterIfVisible(tag("onboarding.storage.hardware"))
        }

        return this
    }

    fun chooseSoftwareWallet(): FullLaunchOnboardingRobot {
        device.advanceUntilVisible(text("Import your software wallet")) {
            clickCenterIfVisible(tag("onboarding.storage.software"))
        }

        return this
    }

    fun chooseSoftwareCreate(): FullLaunchOnboardingRobot {
        device.advanceUntilVisible(text("Creating your wallet")) {
            clickCenterIfVisible(tag("onboarding.software.create"))
        }
        device.waitUntilVisible(textContains("Back up your wallet"), timeoutMillis = 10_000)

        return this
    }

    fun assertBackupWallet(): FullLaunchOnboardingRobot {
        device.waitUntilVisible(textContains("Back up your wallet"))
        device.waitUntilVisible(tag("onboarding.secretWords"))
        device.waitUntilVisible(tag("onboarding.cloudBackup.prompt"))

        return this
    }

    fun viewRecoveryWords(): FullLaunchOnboardingRobot {
        device.waitUntilVisible(text("Show Words")).click()
        device.waitUntilVisible(text("Your Recovery Words"))
        device.waitUntilVisible(tag("onboarding.secretWords.saved"))

        return this
    }

    fun assertScreenshotsBlocked(): FullLaunchOnboardingRobot {
        assertFlagSecureSet(true, "expected screenshots to be blocked")

        return this
    }

    fun assertScreenshotsAllowed(): FullLaunchOnboardingRobot {
        assertFlagSecureSet(false, "expected screenshots to be allowed")

        return this
    }

    fun saveRecoveryWords(): FullLaunchOnboardingRobot {
        viewRecoveryWords()
        device.waitUntilVisible(tag("onboarding.secretWords.saved")).click()
        device.waitUntilVisible(textContains("Back up your wallet"))
        device.waitUntilVisible(text("Saved"))

        return this
    }

    fun continueFromBackupWallet(): FullLaunchOnboardingRobot {
        device.waitUntilVisible(tag("onboarding.continue")).click()

        return this
    }

    fun assertExchangeFunding(): FullLaunchOnboardingRobot {
        device.waitUntilVisible(text("Your wallet is ready to fund"))
        device.waitUntilAnyVisible(text("Loading deposit address"), text("Deposit address"), timeoutMillis = 15_000)

        return this
    }

    fun assertHardwareImportChoices(): FullLaunchOnboardingRobot {
        device.advanceUntilVisible(text("Import your hardware wallet"))
        device.waitUntilVisible(text("Scan export QR"))
        device.waitUntilVisible(text("Import export file"))
        device.waitUntilVisible(text("Scan with NFC"))

        return this
    }

    fun openHardwareQrScanner(): FullLaunchOnboardingRobot {
        device.clickCenter(device.advanceUntilVisible(text("Scan export QR")))
        device.waitUntilVisible(text("Scan Hardware QR"))
        assertQrScannerVisible()

        return this
    }

    fun openHardwareNfcScanner(): FullLaunchOnboardingRobot {
        device.clickCenter(device.advanceUntilVisible(text("Scan with NFC")))
        device.waitUntilVisible(text("Scan your hardware wallet with NFC"))
        device.waitUntilVisible(text("Start NFC Scan"))

        return this
    }

    fun assertSoftwareImportChoices(): FullLaunchOnboardingRobot {
        device.advanceUntilVisible(text("Import your software wallet"))
        device.waitUntilVisible(text("Enter recovery words"))
        device.waitUntilVisible(text("Scan QR code"))

        return this
    }

    fun openSoftwareQrScanner(): FullLaunchOnboardingRobot {
        device.clickCenter(device.advanceUntilVisible(text("Scan QR code")))
        assertQrScannerVisible()

        return this
    }

    fun importKnownEmptyMainnetWalletWords(): FullLaunchOnboardingRobot {
        device.advanceUntilVisible(text("How many words do you have?")) {
            clickCenterIfVisible(text("Enter recovery words"))
        }
        device.advanceUntilVisible(text("Import Wallet")) {
            clickCenterIfVisible(text("12 words"))
        }

        knownEmptyMainnetMnemonic.forEachIndexed { index, word ->
            device.waitUntilVisible(tag("hotWalletImport.word.${index + 1}")).text = word
        }

        device.dismissKeyboardIfShown()
        device.clickUntilVisible(
            clickSelector = tag("hotWalletImport.import"),
            targetSelector = text("Protect this wallet with Cloud Backup?"),
            timeoutMillis = 20_000,
        )

        return this
    }

    fun assertImportScreenBlocksScreenshots(): FullLaunchOnboardingRobot {
        device.advanceUntilVisible(text("How many words do you have?")) {
            clickCenterIfVisible(text("Enter recovery words"))
        }
        device.advanceUntilVisible(text("Import Wallet")) {
            clickCenterIfVisible(text("12 words"))
        }
        assertScreenshotsBlocked()

        return this
    }

    fun skipCloudBackupAfterImport(): FullLaunchOnboardingRobot {
        device.waitUntilVisible(text("Not Now")).click()

        return this
    }

    fun acceptTermsAfterImport(): FullLaunchOnboardingRobot {
        device.waitUntilVisible(text("Terms & Conditions"))
        device.acceptTerms()

        return this
    }

    fun chooseNativeImportedWalletFromSelectionSheet(): FullLaunchOnboardingRobot {
        device.waitUntilVisible(text("Multiple wallets found, please choose one"), timeoutMillis = 30_000)
        device.waitUntilVisible(text("Keep Current"))
        device.waitUntilVisible(text("Wrapped Segwit"))
        device.waitUntilVisible(text("Legacy"))
        device.waitUntilVisible(text("Keep Current")).click()
        device.waitUntilVisible(text("Send"), timeoutMillis = 30_000)

        return this
    }

    fun assertImportedMainnetWalletHasHistoryAndNoBitcoin(): FullLaunchOnboardingRobot {
        device.waitUntilSendShowsNoFunds()
        device.waitUntilVisible(text("Transactions"), timeoutMillis = 30_000)

        return this
    }

    fun openCloudBackupFromBackupWallet(): FullLaunchOnboardingRobot {
        device.waitUntilVisible(tag("onboarding.cloudBackup.prompt"))
        device.waitUntilVisible(text("Enable")).click()
        device.waitUntilVisible(text("Cloud Backup"))

        return this
    }

    fun cancelCloudBackupDetails(): FullLaunchOnboardingRobot {
        device.waitUntilVisible(tag("onboarding.cloudBackup.cancel")).click()

        return this
    }

    fun systemBackFromCloudBackupDetails(): FullLaunchOnboardingRobot {
        device.pressBack()

        return this
    }

    fun assertCloudBackupDetails(): FullLaunchOnboardingRobot {
        device.waitUntilVisible(text("Cloud Backup"))
        device.waitUntilVisible(text("How It Works"))
        device.waitUntilVisible(text("Back"))
        device.waitUntilVisible(tag("onboarding.cloudBackup.cancel"))

        return this
    }

    fun enableCloudBackupFromDetails(): FullLaunchOnboardingRobot {
        listOf(
            "my passkey is required",
            "access to my Google account",
            "manually back up my 12 or 24 words",
        ).forEach { label ->
            device.clickCheckboxBesideLabel(label)
        }

        device.clickObjectUntilGoneOrSystemSheet(tag("onboarding.cloudBackup.enable"))

        return this
    }

    fun assertCreatePasskeySheetShown(): FullLaunchOnboardingRobot {
        val wrongSheetSelectors =
            listOf(
                textContains("Sign in another way"),
                textContains("Use passkey from"),
            )
        val createSheetSelectors =
            listOf(
                textContains("Create a passkey"),
                textContains("Create passkey"),
                textContains("Save a passkey"),
                textContains("Save passkey"),
            )
        val deadline = System.currentTimeMillis() + 20_000

        while (System.currentTimeMillis() < deadline) {
            selectGoogleAuthorizationAccountIfNeeded()
            chooseNewPasskeyIfExistingBackupPromptShown()

            if (createSheetSelectors.any { device.findObject(it.value) != null }) {
                device.pressBack()
                return this
            }

            wrongSheetSelectors.firstOrNull { device.findObject(it.value) != null }?.let { selector ->
                fail("Expected passkey creation sheet, but saw ${selector.description}\n${device.dumpWindowHierarchy()}")
            }

            Thread.sleep(100)
        }

        error("Timed out waiting for passkey creation sheet\n${device.dumpWindowHierarchy()}")
    }

    private fun chooseNewPasskeyIfExistingBackupPromptShown() {
        if (device.findObject(textContains("previous backup").value) == null) {
            return
        }

        val createButton =
            device.findObject(text("Create New Backup").value)
                ?: device.findObject(text("Create New Passkey").value)
                ?: return

        val bounds = createButton.visibleBounds
        device.click(bounds.centerX(), bounds.centerY())
        Thread.sleep(500)
    }

    private fun selectGoogleAuthorizationAccountIfNeeded() {
        if (device.findObject(text("Choose an account").value) == null) {
            return
        }

        device.findObject(textContains("@").value)?.click()
        Thread.sleep(500)
    }

    private fun assertQrScannerVisible() {
        device.waitUntilAnyVisible(text("1x"), text("Camera Access Required"))
    }
}

fun fullLaunchDevice(): UiDevice =
    UiDevice.getInstance(InstrumentationRegistry.getInstrumentation()).apply {
        ensureManualFullLaunchDeviceReady()
    }

fun launchFullApp() {
    val instrumentation = InstrumentationRegistry.getInstrumentation()
    val packageName = instrumentation.targetContext.packageName
    val device = UiDevice.getInstance(instrumentation)

    device.ensureManualFullLaunchDeviceReady()

    val output =
        instrumentation.uiAutomation.executeShellCommand(
            "am start -W -n $packageName/org.bitcoinppl.cove.MainActivity --ez org.bitcoinppl.cove.uitest.RESET_DATA true",
        )

    output.drainAndClose()
    device.ensureManualFullLaunchDeviceReady()
}

private fun UiDevice.ensureManualFullLaunchDeviceReady(timeoutMillis: Long = 5_000) {
    val deadline = System.currentTimeMillis() + timeoutMillis

    while (System.currentTimeMillis() < deadline) {
        wakeUp()
        executeShellCommand("wm dismiss-keyguard")

        if (!isDeviceLocked()) {
            return
        }

        Thread.sleep(250)
    }

    fail(
        "Manual full-launch tests require the connected Android device to be unlocked. " +
            "Unlock the device and rerun just ui-manual.\n${dumpWindowHierarchy()}",
    )
}

private fun UiDevice.isDeviceLocked(): Boolean {
    val trustDump = executeShellCommand("dumpsys trust")
    val currentUserLine = trustDump.lineSequence().firstOrNull { "(current)" in it }

    if (currentUserLine != null) {
        return "deviceLocked=1" in currentUserLine
    }

    return "showingAndNotOccluded=true" in executeShellCommand("dumpsys window policy")
}

private fun assertFlagSecureSet(
    expected: Boolean,
    message: String,
    timeoutMillis: Long = 500,
    intervalMillis: Long = 50,
) {
    val deadline = System.currentTimeMillis() + timeoutMillis

    while (System.currentTimeMillis() < deadline) {
        if (isFlagSecureSet() == expected) {
            return
        }

        Thread.sleep(intervalMillis)
    }

    assertTrue(message, isFlagSecureSet() == expected)
}

private fun isFlagSecureSet(): Boolean {
    val activity = resumedActivity()

    return (activity.window.attributes.flags and WindowManager.LayoutParams.FLAG_SECURE) != 0
}

private fun resumedActivity(): android.app.Activity {
    var activity: android.app.Activity? = null
    InstrumentationRegistry.getInstrumentation().runOnMainSync {
        activity =
            ActivityLifecycleMonitorRegistry
                .getInstance()
                .getActivitiesInStage(Stage.RESUMED)
                .firstOrNull()
    }

    return activity ?: error("No resumed activity")
}

private fun UiDevice.waitUntilVisible(
    selector: DescribedSelector,
    timeoutMillis: Long = 20_000,
): UiObject2 =
    wait(Until.findObject(selector.value), timeoutMillis)
        ?: error("Timed out waiting for ${selector.description}\n${dumpWindowHierarchy()}")

private fun UiDevice.waitUntilAnyVisible(
    first: DescribedSelector,
    second: DescribedSelector,
    timeoutMillis: Long = 20_000,
): VisibleSelector {
    val deadline = System.currentTimeMillis() + timeoutMillis

    while (System.currentTimeMillis() < deadline) {
        findObject(first.value)?.let { return VisibleSelector(first.description, it) }
        findObject(second.value)?.let { return VisibleSelector(second.description, it) }
        Thread.sleep(100)
    }

    error("Timed out waiting for ${first.description} or ${second.description}\n${dumpWindowHierarchy()}")
}

private fun UiDevice.advanceUntilVisible(
    targetSelector: DescribedSelector,
    timeoutMillis: Long = 20_000,
    advance: UiDevice.() -> Boolean = { false },
): UiObject2 {
    val deadline = System.currentTimeMillis() + timeoutMillis
    var didAdvance = false

    while (System.currentTimeMillis() < deadline) {
        findObject(targetSelector.value)?.let { return it }

        if (continuePastCloudRestorePromptIfShown()) {
            waitForIdle()
            Thread.sleep(250)
            continue
        }

        if (!didAdvance) {
            didAdvance = advance()
        }
        waitForIdle()
        Thread.sleep(250)
    }

    error("Timed out waiting for ${targetSelector.description}\n${dumpWindowHierarchy()}")
}

private fun UiDevice.continuePastCloudRestorePromptIfShown(): Boolean {
    listOf(
        text("Continue setup"),
        text("Set Up as New"),
        text("Continue Without Backup"),
        text("Continue Without Cloud Restore"),
    ).firstNotNullOfOrNull { selector -> findObject(selector.value) }?.let { button ->
        clickCenter(button)

        return true
    }

    if (isCloudRestorePromptVisible()) {
        pressBack()
        return true
    }

    return false
}

private fun UiDevice.scrollUntilVisible(selector: DescribedSelector): UiObject2 {
    tryScrollUntilVisible(selector)?.let { return it }

    return waitUntilVisible(selector)
}

private fun UiDevice.tryScrollUntilVisible(selector: DescribedSelector): UiObject2? {
    repeat(8) {
        findObject(selector.value)?.let { return it }
        swipe(displayWidth / 2, displayHeight * 3 / 4, displayWidth / 2, displayHeight / 4, 20)
    }

    return findObject(selector.value)
}

private fun UiDevice.isCloudRestorePromptVisible(): Boolean =
    findObject(text("Google Drive Backup Found").value) != null ||
        findObject(text("Restore from Google Drive").value) != null ||
        findObject(text("No Google Drive Backup Found").value) != null ||
        findObject(text("You're Offline").value) != null ||
        findObject(text("Cove backup found").value) != null

private fun UiDevice.clickUntilVisible(
    clickSelector: DescribedSelector,
    targetSelector: DescribedSelector,
    timeoutMillis: Long,
) {
    val deadline = System.currentTimeMillis() + timeoutMillis

    while (System.currentTimeMillis() < deadline) {
        findObject(targetSelector.value)?.let { return }

        val clickNode = tryScrollUntilSafelyClickable(clickSelector)
        if (clickNode?.isEnabled == true) {
            clickCenter(clickNode)
        }

        Thread.sleep(500)
    }

    error("Timed out waiting for ${targetSelector.description}\n${dumpWindowHierarchy()}")
}

private fun UiDevice.clickObjectUntilGoneOrSystemSheet(selector: DescribedSelector) {
    val deadline = System.currentTimeMillis() + 10_000
    var clicked = false

    while (System.currentTimeMillis() < deadline) {
        if (findObject(textContains("Create a passkey").value) != null ||
            findObject(textContains("Create passkey").value) != null ||
            findObject(textContains("Save a passkey").value) != null ||
            findObject(textContains("Save passkey").value) != null ||
            findObject(text("Choose an account").value) != null ||
            findObject(textContains("previous backup").value) != null ||
            findObject(textContains("Creating your passkey").value) != null
        ) {
            return
        }

        val button = tryScrollUntilSafelyClickable(selector)
            ?: if (clicked) {
                return
            } else {
                error("Timed out waiting for ${selector.description}\n${dumpWindowHierarchy()}")
            }
        waitForIdle()
        button.click()
        clicked = true
        Thread.sleep(500)
    }

    error("Timed out waiting for ${selector.description} to start an operation\n${dumpWindowHierarchy()}")
}

private fun UiDevice.acceptTerms() {
    listOf(
        "onboarding.terms.check.backup",
        "onboarding.terms.check.legal",
        "onboarding.terms.check.financial",
        "onboarding.terms.check.recovery",
        "onboarding.terms.check.agreement",
    ).forEach { tag ->
        clickLeadingEdge(scrollUntilVisible(tag(tag)))
    }

    clickSafely(tag("onboarding.terms.agree"))
}

private fun UiDevice.dismissKeyboardIfShown() {
    clickKeyboardDoneIfShown()
    Thread.sleep(250)

    if (!isKeyboardDoneVisible()) {
        return
    }

    pressBack()
    Thread.sleep(250)
}

private fun UiDevice.clickKeyboardDoneIfShown(): Boolean {
    val doneKey =
        findObject(desc("Enter").value)
            ?: findObject(text("Done").value)
            ?: return false

    clickCenter(doneKey)

    return true
}

private fun UiDevice.isKeyboardDoneVisible(): Boolean =
    findObject(desc("Enter").value) != null || findObject(text("Done").value) != null

private fun UiDevice.clickLeadingEdge(node: UiObject2) {
    val bounds = node.visibleBounds
    val x = (bounds.left + 60).coerceAtMost(bounds.right - 1)

    click(x, bounds.centerY())
}

private fun UiDevice.clickCheckboxBesideLabel(label: String) {
    val labelNode = scrollUntilVisible(textContains(label))
    clickCenter(labelNode)
}

private fun UiDevice.clickSafely(selector: DescribedSelector) {
    clickCenter(scrollUntilSafelyClickable(selector))
}

private fun UiDevice.scrollUntilSafelyClickable(selector: DescribedSelector): UiObject2 {
    tryScrollUntilSafelyClickable(selector)?.let { return it }

    return waitUntilVisible(selector)
}

private fun UiDevice.tryScrollUntilSafelyClickable(selector: DescribedSelector): UiObject2? {
    repeat(12) {
        findObject(selector.value)?.let { node ->
            if (isSafelyClickable(node)) return node
        }
        swipe(displayWidth / 2, displayHeight * 3 / 4, displayWidth / 2, displayHeight / 4, 20)
    }

    return findObject(selector.value)?.takeIf { isSafelyClickable(it) }
}

private fun UiDevice.isSafelyClickable(node: UiObject2): Boolean {
    val bounds = node.visibleBounds
    val safeBottom = displayHeight - 200

    return bounds.width() > 0 &&
        bounds.height() > 0 &&
        bounds.centerX() in 1 until displayWidth &&
        bounds.centerY() in 1 until safeBottom
}

private fun UiDevice.clickCenter(node: UiObject2) {
    val bounds = node.visibleBounds

    click(bounds.centerX(), bounds.centerY())
}

private fun UiDevice.clickCenterIfVisible(selector: DescribedSelector): Boolean {
    findObject(selector.value)?.let { node ->
        clickCenter(node)

        return true
    }

    return false
}

private fun UiDevice.waitUntilSendShowsNoFunds() {
    val noFundsSelector = text("No funds available to send")
    val initialScanIncompleteSelector = text("Initial Scan Incomplete")
    val deadline = System.currentTimeMillis() + 90_000

    while (System.currentTimeMillis() < deadline) {
        clickCenter(waitUntilVisible(text("Send"), timeoutMillis = 30_000))

        val sendUnavailable = waitUntilAnyVisible(noFundsSelector, initialScanIncompleteSelector, timeoutMillis = 10_000)
        if (sendUnavailable.description == noFundsSelector.description) {
            return
        }

        waitUntilVisible(text("Can't send until initial scan completes."))
        clickCenter(waitUntilVisible(text("OK")))
        waitForIdle()
        Thread.sleep(2_000)
    }

    error("Timed out waiting for Send to report no funds after scan completion\n${dumpWindowHierarchy()}")
}

private class DescribedSelector(
    val value: BySelector,
    val description: String,
)

private class VisibleSelector(
    val description: String,
    val node: UiObject2,
)

private fun tag(value: String): DescribedSelector =
    DescribedSelector(By.res(value), "tag \"$value\"")

private fun text(value: String): DescribedSelector =
    DescribedSelector(By.text(value), "text \"$value\"")

private fun textContains(value: String): DescribedSelector =
    DescribedSelector(By.textContains(value), "text containing \"$value\"")

private fun desc(value: String): DescribedSelector =
    DescribedSelector(By.desc(value), "content description \"$value\"")

private fun UiDevice.dumpWindowHierarchy(): String {
    val output = ByteArrayOutputStream()
    dumpWindowHierarchy(output)

    return output.toString(Charsets.UTF_8.name()).take(120_000)
}

private fun ParcelFileDescriptor.drainAndClose() {
    ParcelFileDescriptor.AutoCloseInputStream(this).bufferedReader().use { it.readText() }
}
