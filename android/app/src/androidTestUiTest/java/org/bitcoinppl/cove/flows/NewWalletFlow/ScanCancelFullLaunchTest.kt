package org.bitcoinppl.cove.flows.NewWalletFlow

import android.Manifest
import android.content.pm.PackageManager
import android.system.Os
import androidx.core.content.ContextCompat
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
import org.bitcoinppl.cove.test.HardwareSignerControl
import org.bitcoinppl.cove.test.HardwareWalletScreens
import org.bitcoinppl.cove.test.NfcNdefScript
import org.bitcoinppl.cove.test.PublicKeyAddressType
import org.bitcoinppl.cove.test.QrScript
import org.bitcoinppl.cove.test.fullLaunchDevice
import org.bitcoinppl.cove.test.launchFullApp
import org.bitcoinppl.cove.test.waitFor
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

/**
 * Cancels the camera permission dialog and the NFC scan sheet in the real app, and checks neither
 * cancel goes on to scan
 *
 * The camera test opens onboarding's hardware QR scanner without the debug QR seam, so the app asks
 * for the camera itself and the system permission dialog answers. The NFC test reads tags through
 * the debug NFC seam, which replaces only the adapter. Runs in the uiTest flavour; needs the fake
 * node and the hardware signer host up and forwarded, which `just android-instrumented-ci` does
 */
@LargeTest
@RunWith(AndroidJUnit4::class)
class ScanCancelFullLaunchTest {
    @get:Rule
    val stayAwake = AndroidDeviceStayAwakeRule()

    private lateinit var device: UiDevice
    private lateinit var wallet: FullLaunchWalletRobot
    private lateinit var screens: HardwareWalletScreens
    private val node = FakeNodeControl()
    private val signer = HardwareSignerControl()
    private val packageName = InstrumentationRegistry.getInstrumentation().targetContext.packageName
    private var qr: QrScript? = null
    private var nfc: NfcNdefScript? = null
    private var previousShowImeWithHardKeyboard: String? = null

    @Before
    fun prepareDevice() {
        device = fullLaunchDevice()
        wallet = FullLaunchWalletRobot(device)
        screens = HardwareWalletScreens(device, wallet)

        // see UtxoLockFullLaunchTest: the onboarding robot must never see the soft keyboard
        previousShowImeWithHardKeyboard = device.executeShellCommand("settings get secure show_ime_with_hard_keyboard").trim()
        device.executeShellCommand("settings put secure show_ime_with_hard_keyboard 0")
    }

    @After
    fun restoreDevice() {
        qr?.remove()
        nfc?.remove()

        // a denial is remembered, and a second one stops Android asking at all, so clear it for
        // the next run; revoking or granting would kill this process, which hosts the test
        device.executeShellCommand("pm clear-permission-flags $packageName ${Manifest.permission.CAMERA} user-set user-fixed")

        when (val previous = previousShowImeWithHardKeyboard) {
            null, "null" -> device.executeShellCommand("settings delete secure show_ime_with_hard_keyboard")
            else -> device.executeShellCommand("settings put secure show_ime_with_hard_keyboard $previous")
        }
    }

    /**
     * Denying the camera in the system dialog keeps the app's Camera Access Required screen up with
     * no scanner behind it, and Cancel there goes back to where the scan started
     */
    @Test
    fun deniedCameraKeepsTheScannerClosedAndCancelGoesBack() {
        // a variable left from another test in this process would replace the camera
        Os.unsetenv("COVE_UI_TEST_QR_SCRIPT")
        assertFalse(
            "the camera is already granted, so the system dialog can't be denied",
            cameraGranted(),
        )

        startOnboardingHardwareImport().openHardwareQrScanner()
        device.waitFor(By.text("Camera Access Required"))
        device.waitFor(By.text("Grant Permission")).click()

        val deny =
            device.wait(Until.findObject(By.res(PERMISSION_CONTROLLER, "permission_deny_button")), 10_000L)
                ?: device.waitFor(By.text("Don’t allow"))
        deny.click()
        assertTrue(
            "the permission dialog stayed after Don't allow",
            device.wait(Until.gone(By.pkg(PERMISSION_CONTROLLER)), 10_000L),
        )

        // gives a scanner that would open after the denial time to show its zoom control
        assertFalse("a scanner opened after the camera was denied", device.wait(Until.hasObject(By.text("1x")), 3_000L))
        assertFalse("the camera should still be denied", cameraGranted())
        assertTrue("the app should stay on its camera screen", device.hasObject(By.text("Camera Access Required")))
        assertTrue("Grant Permission should stay to ask again", device.hasObject(By.text("Grant Permission")))

        device.waitFor(By.text("Cancel")).click()
        device.waitFor(By.text("Scan export QR"))
        assertFalse("Cancel should close the camera screen", device.hasObject(By.text("Camera Access Required")))
    }

    /**
     * A dismissed NFC scan sheet stops reading, so a tag held to the phone afterwards imports
     * nothing, and the sheet opened again reads it
     */
    @Test
    fun dismissedNfcSheetReadsNoTagUntilItOpensAgain() {
        signer.requireHealthy()
        node.reset()
        val first = signer.publicKeyExport(PublicKeyAddressType.NATIVE_SEGWIT)
        val second = signer.publicKeyExport(PublicKeyAddressType.WRAPPED_SEGWIT)
        val qr = QrScript().also { qr = it }
        val nfc = NfcNdefScript().also { nfc = it }

        // Add Wallet is in the sidebar once the app has a wallet
        startOnboardingHardwareImport().openHardwareQrScanner()
        qr.show(listOf(first.key))
        finishOnboardingImport()

        screens.scanNfcFromAddWallet()
        device.waitFor(By.text("Ready to Scan"))
        device.pressBack()
        assertTrue("Back should close the NFC sheet", device.wait(Until.gone(By.text("Ready to Scan")), 10_000L))
        device.waitFor(By.text("Add New Wallet"))

        // the seam polls every 200 ms while a scan runs, so a scan still running takes the tag
        // well within this
        nfc.showTextRecord(second.key)
        Thread.sleep(3_000L)
        assertTrue("the dismissed NFC scan read the tag", nfc.isWaiting)
        assertFalse("a tag imported after the NFC sheet closed", device.hasObject(By.text("Wallet imported successfully")))

        // Back closed only the sheet, so Add Wallet is still open to start the scan again
        device.waitFor(By.desc("More wallet options")).click()
        device.waitFor(By.text("NFC")).click()
        device.waitFor(By.text("Wallet imported successfully"), timeoutMs = 30_000L)
        device.waitFor(By.text("OK")).click()
        wallet.waitForBalance(0L)

        // the import is the tag's account, not a second copy of the wallet already there
        screens.openReceive()
        device.waitFor(By.text("Derivation: ${second.firstDerivationPath}"))
        assertEquals("the reopened sheet imported the wrong key", second.firstAddress, screens.copyOpenReceiveAddress())
    }

    private fun cameraGranted(): Boolean {
        val context = InstrumentationRegistry.getInstrumentation().targetContext
        return ContextCompat.checkSelfPermission(context, Manifest.permission.CAMERA) == PackageManager.PERMISSION_GRANTED
    }

    /** Launches a fresh install on the fake node and opens onboarding's hardware wallet choices */
    private fun startOnboardingHardwareImport(): FullLaunchOnboardingRobot {
        launchFullApp()
        FullLaunchStartupRobot(device).assertBootstrappedIntoOnboarding()
        wallet.selectSignetFakeNode()

        return FullLaunchOnboardingRobot(device)
            .tapGetStarted()
            .chooseExistingUser()
            .useAnotherWallet()
            .chooseHardwareWallet()
    }

    /** Declines Cloud Backup and accepts the terms after an onboarding import */
    private fun finishOnboardingImport() {
        device.waitFor(By.text("Protect this hardware wallet with Cloud Backup?"), timeoutMs = 30_000L)
        FullLaunchOnboardingRobot(device)
            .skipCloudBackupAfterImport()
            .acceptTermsAfterImport()
        wallet.waitForBalance(0L)
    }

    private companion object {
        const val PERMISSION_CONTROLLER = "com.android.permissioncontroller"
    }
}
