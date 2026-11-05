package org.bitcoinppl.cove.flows.NewWalletFlow

import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.filters.LargeTest
import androidx.test.uiautomator.By
import androidx.test.uiautomator.UiDevice
import org.bitcoinppl.cove.test.AndroidDeviceStayAwakeRule
import org.bitcoinppl.cove.test.FakeNodeControl
import org.bitcoinppl.cove.test.FullLaunchOnboardingRobot
import org.bitcoinppl.cove.test.FullLaunchStartupRobot
import org.bitcoinppl.cove.test.FullLaunchWalletRobot
import org.bitcoinppl.cove.test.HardwareSignerControl
import org.bitcoinppl.cove.test.HardwareWalletScreens
import org.bitcoinppl.cove.test.NfcNdefScript
import org.bitcoinppl.cove.test.PublicKeyAddressType
import org.bitcoinppl.cove.test.PublicKeyExport
import org.bitcoinppl.cove.test.QrScript
import org.bitcoinppl.cove.test.fullLaunchDevice
import org.bitcoinppl.cove.test.launchFullApp
import org.bitcoinppl.cove.test.waitFor
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

/**
 * Imports a hardware wallet from its bare public key in the real app, scanned through the debug QR
 * seam as a plain QR and as a single-part BBQr, or read through the debug NFC seam from a tag's text
 * record, and checks the address type the app infers from the key by the first receive address it
 * shows
 *
 * The signer host derives the key and the expected address from the test mnemonic, the address
 * straight from the key at `/0/0` rather than from a descriptor. The fake node is reset first, so
 * the account has no history and Receive opens at index 0. Runs in the uiTest flavour; needs the
 * fake node and the hardware signer host up and forwarded, which `just android-instrumented-ci` does
 */
@LargeTest
@RunWith(AndroidJUnit4::class)
class PublicKeyImportFullLaunchTest {
    @get:Rule
    val stayAwake = AndroidDeviceStayAwakeRule()

    private lateinit var device: UiDevice
    private lateinit var wallet: FullLaunchWalletRobot
    private lateinit var screens: HardwareWalletScreens
    private lateinit var qr: QrScript
    private lateinit var nfc: NfcNdefScript
    private val node = FakeNodeControl()
    private val signer = HardwareSignerControl()
    private var previousShowImeWithHardKeyboard: String? = null

    @Before
    fun prepareDevice() {
        device = fullLaunchDevice()
        wallet = FullLaunchWalletRobot(device)
        screens = HardwareWalletScreens(device, wallet)
        qr = QrScript()
        nfc = NfcNdefScript()

        // see UtxoLockFullLaunchTest: the onboarding robot must never see the soft keyboard
        previousShowImeWithHardKeyboard = device.executeShellCommand("settings get secure show_ime_with_hard_keyboard").trim()
        device.executeShellCommand("settings put secure show_ime_with_hard_keyboard 0")
    }

    @After
    fun restoreDevice() {
        qr.remove()
        nfc.remove()

        when (val previous = previousShowImeWithHardKeyboard) {
            null, "null" -> device.executeShellCommand("settings delete secure show_ime_with_hard_keyboard")
            else -> device.executeShellCommand("settings put secure show_ime_with_hard_keyboard $previous")
        }
    }

    /** A vpub names a BIP84 account, so it imports as native segwit */
    @Test
    fun plainVpubQrImportsNativeSegwitWithItsFirstAddress() {
        val export = publicKeyExport(PublicKeyAddressType.NATIVE_SEGWIT)
        assertTrue(export.key.startsWith("vpub"))

        importAndCheckFirstAddress(export, listOf(export.key))
    }

    /** A upub names a BIP49 account, so it imports as wrapped segwit, not the native segwit default */
    @Test
    fun singleBbqrUpubImportsWrappedSegwitWithItsFirstAddress() {
        val export = publicKeyExport(PublicKeyAddressType.WRAPPED_SEGWIT)
        assertTrue(export.key.startsWith("upub"))

        // a BBQr header is B$, the encoding and file type, then the part count and index
        assertEquals("the export must be a single BBQr part", "0100", export.bbqr.substring(4, 8))

        importAndCheckFirstAddress(export, listOf(export.bbqr))
    }

    /**
     * A vpub in a tag's text record imports from onboarding's NFC scan, where a new user first holds
     * a tag to the phone. The record's status byte and language code must not reach the key
     */
    @Test
    fun nfcTextRecordVpubImportsFromOnboarding() {
        val export = publicKeyExport(PublicKeyAddressType.NATIVE_SEGWIT)
        assertTrue(export.key.startsWith("vpub"))

        startOnboardingHardwareImport().openHardwareNfcScanner()
        device.waitFor(By.text("Start NFC Scan")).click()
        nfc.showTextRecord(export.key)

        finishOnboardingImport()
        checkFirstAddress(export)
    }

    /** A upub in a tag's text record imports from Add Wallet's NFC scan sheet as wrapped segwit */
    @Test
    fun nfcTextRecordUpubImportsFromAddWallet() {
        val first = publicKeyExport(PublicKeyAddressType.NATIVE_SEGWIT)
        val export = signer.publicKeyExport(PublicKeyAddressType.WRAPPED_SEGWIT)
        assertTrue(export.key.startsWith("upub"))

        // Add Wallet is in the sidebar once the app has a wallet
        startOnboardingHardwareImport().openHardwareQrScanner()
        qr.show(listOf(first.key))
        finishOnboardingImport()

        screens.scanNfcFromAddWallet()
        nfc.showTextRecord(export.key)

        device.waitFor(By.text("Wallet imported successfully"), timeoutMs = 30_000L)
        device.waitFor(By.text("OK")).click()
        wallet.waitForBalance(0L)

        checkFirstAddress(export)
    }

    private fun publicKeyExport(addressType: PublicKeyAddressType): PublicKeyExport {
        signer.requireHealthy()
        node.reset()

        return signer.publicKeyExport(addressType)
    }

    private fun importAndCheckFirstAddress(
        export: PublicKeyExport,
        frames: List<String>,
    ) {
        startOnboardingHardwareImport().openHardwareQrScanner()
        qr.show(frames)

        finishOnboardingImport()
        checkFirstAddress(export)
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

    private fun checkFirstAddress(export: PublicKeyExport) {
        screens.openReceive()

        // the path's purpose is the address type, 84' for native and 49' for wrapped segwit
        device.waitFor(By.text("Derivation: ${export.firstDerivationPath}"))

        assertEquals(
            "the first receive address doesn't match the key's address type",
            export.firstAddress,
            screens.copyOpenReceiveAddress(),
        )
    }
}
