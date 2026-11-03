package org.bitcoinppl.cove.test

import android.content.ClipboardManager
import android.content.Context
import androidx.test.platform.app.InstrumentationRegistry
import androidx.test.uiautomator.By
import androidx.test.uiautomator.BySelector
import androidx.test.uiautomator.StaleObjectException
import androidx.test.uiautomator.UiDevice
import androidx.test.uiautomator.UiObject2
import androidx.test.uiautomator.Until

/** Steps through the Receive and TAPSIGNER screens that several full-launch suites share */
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

    /**
     * Scans a TAPSIGNER from Add Wallet; the NFC seam answers from the emulated card, and the app
     * asks to set it up or import it depending on whether it has a key
     */
    fun scanTapSignerFromAddWallet(alert: String) {
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

        device.waitFor(By.text(alert), timeoutMs = 15_000L)
        device.waitFor(By.text("Yes")).click()
    }

    /**
     * Types the CVC into the TAPSIGNER screen titled [title] and submits it. The TAPSIGNER sheet is
     * its own window, which doesn't expose test tags as resource ids
     *
     * [findSubmit] brings the enabled submit action on screen, so a layout test can check it fits
     * before tapping it
     */
    fun enterTapSignerCvc(
        cvc: String,
        title: String = "Enter TAPSIGNER CVC",
        submit: String = "Submit",
        findSubmit: (BySelector) -> UiObject2 = { device.scrollTo(it) },
    ) {
        device.waitFor(By.text(title), timeoutMs = 15_000L)

        // the field masks its text, so check the digit count beneath it
        wallet.typeInto(By.clazz("android.widget.EditText"), cvc) {
            device.hasObject(By.text("Digits: ${cvc.length}"))
        }
        wallet.dismissKeyboard()
        findSubmit(By.text(submit).enabled(true)).click()
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
