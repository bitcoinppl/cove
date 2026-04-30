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

        device.waitUntilVisible(tag("onboarding.getStarted"))

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
        device.waitUntilVisible(tag("onboarding.getStarted")).click()
        device.waitUntilVisible(text("Do you already have Bitcoin?"))

        return this
    }

    fun chooseExistingUser(): FullLaunchOnboardingRobot {
        device.waitUntilVisible(tag("onboarding.bitcoinChoice.existing")).click()
        device.waitUntilVisible(text("How would you like to continue?"))

        return this
    }

    fun goBackToBitcoinChoice(): FullLaunchOnboardingRobot {
        device.waitUntilVisible(text("Back")).click()
        device.waitUntilVisible(text("Do you already have Bitcoin?"))
        device.waitUntilVisible(tag("onboarding.bitcoinChoice.new"))
        device.waitUntilVisible(tag("onboarding.bitcoinChoice.existing"))

        return this
    }

    fun chooseNewUser(): FullLaunchOnboardingRobot {
        device.waitUntilVisible(tag("onboarding.bitcoinChoice.new")).click()
        device.waitUntilVisible(text("Creating your wallet"))
        device.waitUntilVisible(textContains("Back up your wallet"), timeoutMillis = 10_000)

        return this
    }

    fun useAnotherWallet(): FullLaunchOnboardingRobot {
        device.waitUntilVisible(tag("onboarding.returningUser.anotherWallet")).click()
        device.waitUntilVisible(text("How do you store your Bitcoin?"))

        return this
    }

    fun assertStorageChoices(): FullLaunchOnboardingRobot {
        device.waitUntilVisible(text("How do you store your Bitcoin?"))
        device.waitUntilVisible(tag("onboarding.storage.exchange"))
        device.waitUntilVisible(tag("onboarding.storage.hardware"))
        device.waitUntilVisible(tag("onboarding.storage.software"))

        return this
    }

    fun chooseExchange(): FullLaunchOnboardingRobot {
        device.waitUntilVisible(tag("onboarding.storage.exchange")).click()
        device.waitUntilVisible(text("Creating your wallet"))
        device.waitUntilVisible(text("Back up your wallet before funding it"), timeoutMillis = 10_000)

        return this
    }

    fun chooseHardwareWallet(): FullLaunchOnboardingRobot {
        device.waitUntilVisible(tag("onboarding.storage.hardware")).click()
        device.waitUntilVisible(text("Import your hardware wallet"))

        return this
    }

    fun chooseSoftwareWallet(): FullLaunchOnboardingRobot {
        device.waitUntilVisible(tag("onboarding.storage.software")).click()
        device.waitUntilVisible(text("What would you like to do?"))

        return this
    }

    fun chooseSoftwareCreate(): FullLaunchOnboardingRobot {
        device.waitUntilVisible(tag("onboarding.software.create")).click()
        device.waitUntilVisible(text("Creating your wallet"))
        device.waitUntilVisible(textContains("Back up your wallet"), timeoutMillis = 10_000)

        return this
    }

    fun chooseSoftwareImport(): FullLaunchOnboardingRobot {
        device.waitUntilVisible(tag("onboarding.software.import")).click()
        device.waitUntilVisible(text("Import your software wallet"))

        return this
    }

    fun assertBackupWallet(): FullLaunchOnboardingRobot {
        device.waitUntilVisible(textContains("Back up your wallet"))
        device.waitUntilVisible(tag("onboarding.secretWords"))
        device.waitUntilVisible(tag("onboarding.cloudBackup.prompt"))

        return this
    }

    fun viewRecoveryWords(): FullLaunchOnboardingRobot {
        device.waitUntilVisible(tag("onboarding.secretWords")).click()
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
        device.waitUntilVisible(text("Import your hardware wallet"))
        device.waitUntilVisible(text("Scan export QR"))
        device.waitUntilVisible(text("Import export file"))
        device.waitUntilVisible(text("Scan with NFC"))

        return this
    }

    fun openHardwareQrScanner(): FullLaunchOnboardingRobot {
        device.waitUntilVisible(text("Scan export QR")).click()
        device.waitUntilVisible(text("Scan Hardware QR"))
        assertQrScannerVisible()

        return this
    }

    fun openHardwareNfcScanner(): FullLaunchOnboardingRobot {
        device.waitUntilVisible(text("Scan with NFC")).click()
        device.waitUntilVisible(text("Scan your hardware wallet with NFC"))
        device.waitUntilVisible(text("Start NFC Scan"))

        return this
    }

    fun assertSoftwareImportChoices(): FullLaunchOnboardingRobot {
        device.waitUntilVisible(text("Import your software wallet"))
        device.waitUntilVisible(text("Enter recovery words"))
        device.waitUntilVisible(text("Scan QR code"))

        return this
    }

    fun openSoftwareQrScanner(): FullLaunchOnboardingRobot {
        device.waitUntilVisible(text("Scan QR code")).click()
        assertQrScannerVisible()

        return this
    }

    fun importKnownEmptyMainnetWalletWords(): FullLaunchOnboardingRobot {
        device.waitUntilVisible(text("Enter recovery words")).click()
        device.waitUntilVisible(text("How many words do you have?"))
        device.waitUntilVisible(text("12 words")).click()
        device.waitUntilVisible(text("Import Wallet"))

        knownEmptyMainnetMnemonic.forEachIndexed { index, word ->
            device.waitUntilVisible(tag("hotWalletImport.word.${index + 1}")).text = word
        }

        device.pressEnter()
        device.scrollUntilVisible(tag("hotWalletImport.import")).click()
        device.waitUntilVisible(text("Protect this wallet with Cloud Backup?"), timeoutMillis = 20_000)

        return this
    }

    fun assertImportScreenBlocksScreenshots(): FullLaunchOnboardingRobot {
        device.waitUntilVisible(text("Enter recovery words")).click()
        device.waitUntilVisible(text("How many words do you have?"))
        device.waitUntilVisible(text("12 words")).click()
        device.waitUntilVisible(text("Import Wallet"))
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
        device.waitUntilVisible(text("Send"), timeoutMillis = 30_000).click()
        device.waitUntilVisible(text("No funds available to send"), timeoutMillis = 10_000)
        device.waitUntilVisible(text("Transactions"), timeoutMillis = 30_000)

        return this
    }

    fun openCloudBackupFromBackupWallet(): FullLaunchOnboardingRobot {
        device.waitUntilVisible(tag("onboarding.cloudBackup.prompt")).click()
        device.waitUntilVisible(text("Cloud Backup"))

        return this
    }

    fun cancelCloudBackupDetails(): FullLaunchOnboardingRobot {
        device.waitUntilVisible(tag("onboarding.cloudBackup.cancel")).click()

        return this
    }

    fun assertCloudBackupDetails(): FullLaunchOnboardingRobot {
        device.waitUntilVisible(text("Cloud Backup"))
        device.waitUntilVisible(text("How It Works"))
        device.waitUntilVisible(tag("onboarding.cloudBackup.cancel"))
        device.waitUntilVisible(tag("onboarding.cloudBackup.enable"))

        return this
    }

    private fun assertQrScannerVisible() {
        device.waitUntilAnyVisible(text("1x"), text("Camera Access Required"))
    }
}

fun fullLaunchDevice(): UiDevice =
    UiDevice.getInstance(InstrumentationRegistry.getInstrumentation()).apply {
        wakeUp()
        executeShellCommand("wm dismiss-keyguard")
    }

fun launchFullApp() {
    val instrumentation = InstrumentationRegistry.getInstrumentation()
    val packageName = instrumentation.targetContext.packageName
    val output =
        instrumentation.uiAutomation.executeShellCommand(
            "am start -W -n $packageName/org.bitcoinppl.cove.MainActivity --ez org.bitcoinppl.cove.uitest.RESET_DATA true",
        )

    output.drainAndClose()
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

private fun UiDevice.scrollUntilVisible(selector: DescribedSelector): UiObject2 {
    repeat(8) {
        findObject(selector.value)?.let { return it }
        swipe(displayWidth / 2, displayHeight * 3 / 4, displayWidth / 2, displayHeight / 4, 20)
    }

    return waitUntilVisible(selector)
}

private fun UiDevice.acceptTerms() {
    listOf(
        "onboarding.terms.check.backup",
        "onboarding.terms.check.legal",
        "onboarding.terms.check.financial",
        "onboarding.terms.check.recovery",
        "onboarding.terms.check.agreement",
    ).forEach { tag ->
        scrollUntilVisible(tag(tag)).click()
    }

    scrollUntilVisible(tag("onboarding.terms.agree")).click()
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
