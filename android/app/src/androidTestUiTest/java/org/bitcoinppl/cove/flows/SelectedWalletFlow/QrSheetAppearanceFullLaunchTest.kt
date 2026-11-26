package org.bitcoinppl.cove.flows.SelectedWalletFlow

import android.graphics.Bitmap
import android.view.WindowManager
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.filters.LargeTest
import androidx.test.platform.app.InstrumentationRegistry
import androidx.test.uiautomator.By
import androidx.test.uiautomator.UiDevice
import androidx.test.uiautomator.Until
import org.bitcoinppl.cove.test.AndroidDeviceStayAwakeRule
import org.bitcoinppl.cove.test.FakeNodeControl
import org.bitcoinppl.cove.test.FullLaunchOnboardingRobot
import org.bitcoinppl.cove.test.FullLaunchStartupRobot
import org.bitcoinppl.cove.test.FullLaunchWalletRobot
import org.bitcoinppl.cove.test.HardwareWalletScreens
import org.bitcoinppl.cove.test.fullLaunchDevice
import org.bitcoinppl.cove.test.launchFullApp
import org.bitcoinppl.cove.test.resumedActivity
import org.bitcoinppl.cove.test.waitFor
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

/**
 * Opens QR sheets with the system in dark mode and checks each draws on a light background and
 * raises the screen to full brightness until it closes
 *
 * Cheap scanner cameras, such as SeedSigner's, fail to find a QR code drawn on a dark sheet, so
 * every sheet that shows a QR code for another device stays light whatever the appearance. The
 * test reads the screen beside the QR code, where the sheet's own background shows, and the
 * brightness the app's window asks for. Runs in the uiTest flavour; needs the fake node up and
 * forwarded, which `just android-instrumented-ci` does
 */
@LargeTest
@RunWith(AndroidJUnit4::class)
class QrSheetAppearanceFullLaunchTest {
    private companion object {
        /**
         * Points, as fractions of the screen, in the sheet's side margins, which its content's
         * padding keeps clear, beside the QR code and lower down, inside even the shortest QR sheet
         */
        val SHEET_BACKGROUND = listOf(0.03f to 0.75f, 0.97f to 0.75f, 0.03f to 0.9f)

        /** Every channel above this reads as a light background */
        const val LIGHT_CHANNEL_MIN = 220
    }

    @get:Rule
    val stayAwake = AndroidDeviceStayAwakeRule()

    private lateinit var device: UiDevice
    private lateinit var wallet: FullLaunchWalletRobot
    private lateinit var screens: HardwareWalletScreens
    private val node = FakeNodeControl()
    private var previousNightMode: String? = null

    @Before
    fun prepareDevice() {
        device = fullLaunchDevice()
        wallet = FullLaunchWalletRobot(device)
        screens = HardwareWalletScreens(device, wallet)

        // "Night mode: yes" and the like, set before launch so the app starts dark
        previousNightMode = device.executeShellCommand("cmd uimode night").substringAfter(":").trim()
        device.executeShellCommand("cmd uimode night yes")
    }

    @After
    fun restoreDevice() {
        val previous = previousNightMode ?: return
        device.executeShellCommand("cmd uimode night $previous")
    }

    @Test
    fun qrSheetsStayLightAtFullBrightnessInDarkMode() {
        launchPastOnboarding()
        assertBrightness(WindowManager.LayoutParams.BRIGHTNESS_OVERRIDE_NONE, "before any QR sheet opened")

        screens.openReceive()
        assertSheetIsLight("Receive")
        assertBrightness(WindowManager.LayoutParams.BRIGHTNESS_OVERRIDE_FULL, "with Receive open")

        device.pressBack()
        assertTrue("Receive never closed", device.wait(Until.gone(By.text("Copy Address")), 10_000L))
        assertBrightness(WindowManager.LayoutParams.BRIGHTNESS_OVERRIDE_NONE, "after Receive closed")

        openXpubQrExport()
        assertSheetIsLight("Export Xpub")
        assertBrightness(WindowManager.LayoutParams.BRIGHTNESS_OVERRIDE_FULL, "with Export Xpub open")
    }

    /** Starts a fresh app on the reset fake node and imports a software wallet to finish onboarding */
    private fun launchPastOnboarding() {
        node.reset()

        launchFullApp()
        FullLaunchStartupRobot(device).assertBootstrappedIntoOnboarding()
        wallet.selectSignetFakeNode()
        FullLaunchOnboardingRobot(device)
            .tapGetStarted()
            .chooseExistingUser()
            .useAnotherWallet()
            .chooseSoftwareWallet()
            .importKnownEmptyMainnetWalletWords()
            .skipCloudBackupAfterImport()
            .acceptTermsAfterImport()

        device.waitFor(By.res("selectedWallet.balance"), timeoutMs = 30_000L)
    }

    private fun openXpubQrExport() {
        device.waitFor(By.desc("More")).click()
        device.waitFor(By.text("Export Xpub")).click()
        device.waitFor(By.text("QR Code")).click()
        device.waitFor(By.desc("Decrease density"), timeoutMs = 15_000L)
    }

    /**
     * Waits for the open sheet's background to read light at every point, which it does once the
     * sheet has finished sliding up
     */
    private fun assertSheetIsLight(sheet: String) {
        var shown = sheetBackgroundColors()
        val deadline = System.currentTimeMillis() + 5_000L
        while (!shown.all(::isLight) && System.currentTimeMillis() < deadline) {
            Thread.sleep(250L)
            shown = sheetBackgroundColors()
        }

        SHEET_BACKGROUND.zip(shown).forEach { (point, color) ->
            assertTrue("$sheet draws ${describe(color)} at $point in dark mode, not a light background", isLight(color))
        }
    }

    private fun sheetBackgroundColors(): List<Int> {
        val screenshot = checkNotNull(InstrumentationRegistry.getInstrumentation().uiAutomation.takeScreenshot()) { "no screenshot" }

        // a hardware bitmap's pixels can't be read back directly
        val bitmap = if (screenshot.config == Bitmap.Config.HARDWARE) screenshot.copy(Bitmap.Config.ARGB_8888, false) else screenshot
        return SHEET_BACKGROUND.map { (x, y) ->
            bitmap.getPixel((bitmap.width * x).toInt(), (bitmap.height * y).toInt())
        }
    }

    private fun isLight(color: Int): Boolean = channels(color).all { it > LIGHT_CHANNEL_MIN }

    private fun describe(color: Int): String = channels(color).joinToString(prefix = "rgb(", postfix = ")")

    private fun channels(color: Int) = listOf(color shr 16 and 0xFF, color shr 8 and 0xFF, color and 0xFF)

    /** Waits for the app's window to ask for [expected] brightness, which it sets once composed */
    private fun assertBrightness(
        expected: Float,
        moment: String,
    ) {
        var brightness = windowBrightness()
        val deadline = System.currentTimeMillis() + 5_000L
        while (brightness != expected && System.currentTimeMillis() < deadline) {
            Thread.sleep(100L)
            brightness = windowBrightness()
        }

        assertEquals("the window's brightness $moment", expected, brightness)
    }

    private fun windowBrightness(): Float {
        val activity = resumedActivity()
        var brightness = Float.NaN
        InstrumentationRegistry.getInstrumentation().runOnMainSync {
            brightness = activity.window.attributes.screenBrightness
        }

        return brightness
    }
}
