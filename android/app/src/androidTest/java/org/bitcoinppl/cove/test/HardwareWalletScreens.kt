package org.bitcoinppl.cove.test

import android.content.ClipData
import android.content.ClipboardManager
import android.content.Context
import androidx.test.platform.app.InstrumentationRegistry
import androidx.test.uiautomator.By
import androidx.test.uiautomator.BySelector
import androidx.test.uiautomator.StaleObjectException
import androidx.test.uiautomator.UiDevice
import androidx.test.uiautomator.UiObject2
import androidx.test.uiautomator.Until
import org.junit.Assert.assertTrue

/**
 * Steps through the Receive, TAPSIGNER and QR signing screens that several full-launch suites
 * share
 */
class HardwareWalletScreens(
    private val device: UiDevice,
    private val wallet: FullLaunchWalletRobot,
) {
    /** Opens Receive and copies its address, which closes the sheet */
    fun copyReceiveAddress(): String {
        openReceive()
        return copyOpenReceiveAddress()
    }

    fun openReceive() {
        device.waitFor(By.text("Receive")).click()
        device.waitFor(By.text("Copy Address"))
    }

    fun copyOpenReceiveAddress(): String {
        // Copy Address does nothing until the address loads, and closes the sheet once it copies
        wallet.waitUntil("expected Copy Address to copy the receive address") {
            device.findObject(By.text("Copy Address"))?.click()
            device.wait(Until.gone(By.text("Copy Address")), 2_000L)
        }

        return clipboardText().trim()
    }

    /** The wallet only looks for new transactions on its own now and then, so pull to refresh */
    fun refreshUntilBalance(sats: Long) {
        wallet.waitUntil("expected a balance of $sats sats after refreshing", timeoutMs = 90_000L) {
            if (device.findText(By.res("selectedWallet.balance"))?.let(::parseSats) == sats) return@waitUntil true

            device.swipe(device.displayWidth / 2, device.displayHeight * 2 / 5, device.displayWidth / 2, device.displayHeight * 9 / 10, 30)
            Thread.sleep(3_000)
            false
        }

        wallet.waitForBalance(sats)
    }

    /**
     * Imports the test mnemonic's account as a watch-only hardware wallet by scanning the signer's
     * export, then waits for the funded balance
     */
    fun importWatchOnlyWallet(
        signer: HardwareSignerControl,
        qr: QrScript,
        balanceSats: Long,
    ) {
        launchFullApp()
        FullLaunchStartupRobot(device).assertBootstrappedIntoOnboarding()
        wallet.selectSignetFakeNode()

        FullLaunchOnboardingRobot(device)
            .tapGetStarted()
            .chooseExistingUser()
            .useAnotherWallet()
            .chooseHardwareWallet()
            .openHardwareQrScanner()

        val (external, internal) = signer.watchOnlyDescriptors()
        qr.show(listOf("$external\n$internal"))

        device.waitFor(By.text("Protect this hardware wallet with Cloud Backup?"), timeoutMs = 30_000L)
        FullLaunchOnboardingRobot(device)
            .skipCloudBackupAfterImport()
            .acceptTermsAfterImport()

        wallet.waitForBalance(balanceSats)
    }

    /** Shows the open draft as an animated QR in the format, then copies the unsigned PSBT */
    fun exportUnsignedPsbt(formatLabel: String): String {
        device.waitFor(By.text("Export Transaction")).click()
        device.waitFor(By.text("QR Code")).click()
        device.waitFor(By.text("Scan this QR"), timeoutMs = 15_000L)

        // a small PSBT can fit in one BBQr frame, which shows no frame counter
        device.waitFor(By.text(formatLabel)).click()
        device.waitFor(By.desc("QR code for transaction signing"))

        // the clipboard can still hold text from an earlier step or test, such as a receive address,
        // and the app handles the tap asynchronously, so a read right after it can still see that
        // text; a sentinel shows when Copy has written, even when it writes the same text again
        setClipboard(CLIPBOARD_SENTINEL)
        device.waitFor(By.desc("Copy")).click()
        wallet.waitUntil("the export's Copy never reached the clipboard", timeoutMs = 5_000L) {
            clipboardText() != CLIPBOARD_SENTINEL
        }

        val psbt = clipboardText().trim()
        assertTrue("the Copy button copied nothing", psbt.isNotEmpty())

        device.pressBack()
        device.waitFor(By.text("Import Signature"))

        return psbt
    }

    /** Opens the draft's QR signature scanner, which the QR seam feeds */
    fun openSignatureScanner() {
        device.waitFor(By.text("Import Signature")).click()
        device.waitFor(By.text("QR")).click()
        device.waitFor(By.text("Scan QR Code"))
    }

    /**
     * Scans a TAPSIGNER from Add Wallet; the NFC seam answers from the emulated card, and the app
     * asks to set it up or import it depending on whether it has a key
     */
    fun scanTapSignerFromAddWallet(alert: String) {
        scanNfcFromAddWallet()

        device.waitFor(By.text(alert), timeoutMs = 15_000L)
        device.waitFor(By.text("Yes")).click()
    }

    /** Opens Add Wallet from the sidebar and starts its NFC scan, which the NFC seams answer */
    fun scanNfcFromAddWallet() {
        // a font scale change recreates the activity, which can replace Menu between finding and
        // clicking it
        wallet.waitUntil("expected Menu to open the sidebar") {
            try {
                device.findObject(By.desc("Menu"))?.click()
            } catch (_: StaleObjectException) {
            }

            device.wait(Until.hasObject(By.text("Add Wallet")), 2_000L)
        }

        device.waitFor(By.text("Add Wallet")).click()
        device.waitFor(By.desc("More wallet options")).click()
        device.waitFor(By.text("NFC")).click()
    }

    /**
     * Types the CVC into the TAPSIGNER screen titled [title] and submits it. The TAPSIGNER sheet is
     * its own window, which doesn't expose test tags as resource ids
     *
     * [findSubmit] brings the enabled submit action on screen, so a layout test can check it fits
     * before tapping it. [opened] shows once the screen is open, its title unless a caller that has
     * already checked which screen it is waits for something else, such as the field, when a tall
     * message can leave the title above the top
     */
    fun enterTapSignerCvc(
        cvc: String,
        title: String = "Enter TAPSIGNER CVC",
        submit: String = "Submit",
        opened: BySelector = By.text(title),
        findSubmit: (BySelector) -> UiObject2 = { device.scrollTo(it) },
    ) {
        device.waitFor(opened, timeoutMs = 15_000L)

        // the field masks its text, so check the digit count beneath it
        wallet.typeInto(By.clazz("android.widget.EditText"), cvc) {
            device.hasObject(By.text("Digits: ${cvc.length}"))
        }
        wallet.dismissKeyboard()
        findSubmit(By.text(submit).enabled(true)).click()
    }
}

// what the clipboard holds until the export's Copy replaces it
private const val CLIPBOARD_SENTINEL = "cove-test-clipboard-sentinel"

/** Puts plain text on the device clipboard, as a user copying it elsewhere would */
fun setClipboard(text: String) {
    val instrumentation = InstrumentationRegistry.getInstrumentation()
    instrumentation.runOnMainSync {
        val clipboard = instrumentation.targetContext.getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager
        clipboard.setPrimaryClip(ClipData.newPlainText("signed PSBT", text))
    }
}

/** The text on the device clipboard, which the app's Copy buttons fill */
fun clipboardText(): String {
    var text = ""
    val instrumentation = InstrumentationRegistry.getInstrumentation()
    instrumentation.runOnMainSync {
        val clipboard = instrumentation.targetContext.getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager
        text = clipboard.primaryClip?.getItemAt(0)?.text?.toString().orEmpty()
    }

    return text
}
