package org.bitcoinppl.cove.flows.SelectedWalletFlow

import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.filters.LargeTest
import androidx.test.platform.app.InstrumentationRegistry
import androidx.test.uiautomator.By
import androidx.test.uiautomator.StaleObjectException
import androidx.test.uiautomator.UiDevice
import androidx.test.uiautomator.Until
import org.bitcoinppl.cove.AppManager
import org.bitcoinppl.cove.AppSheetState
import org.bitcoinppl.cove.test.AndroidDeviceStayAwakeRule
import org.bitcoinppl.cove.test.EXTERNAL_ADDRESS
import org.bitcoinppl.cove.test.FakeNodeControl
import org.bitcoinppl.cove.test.FullLaunchOnboardingRobot
import org.bitcoinppl.cove.test.FullLaunchStartupRobot
import org.bitcoinppl.cove.test.FullLaunchWalletRobot
import org.bitcoinppl.cove.test.QrScript
import org.bitcoinppl.cove.test.SECOND_MNEMONIC
import org.bitcoinppl.cove.test.fullLaunchDevice
import org.bitcoinppl.cove.test.hierarchy
import org.bitcoinppl.cove.test.launchFullApp
import org.bitcoinppl.cove.test.waitFor
import org.junit.After
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotEquals
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

/**
 * Checks that the alert for an address scanned from the wallet screen names which of the user's
 * wallets owns it, in the full app against cove-fake-node
 *
 * The first wallet is the test mnemonic and the second the legal winner phrase, scanned in through
 * the debug QR seam, which also stands in for the camera when the addresses are scanned. Runs in
 * the uiTest flavour only, like the other full-launch tests, and needs the fake node up and
 * forwarded, which `just android-instrumented-ci` does
 */
@LargeTest
@RunWith(AndroidJUnit4::class)
class AddressOwnershipScanFullLaunchTest {
    private companion object {
        /** The test mnemonic's first receive address */
        const val FIRST_RECEIVE_0 = "tb1q6rz28mcfaxtmd6v789l9rrlrusdprr9pqcpvkl"

        /** The second mnemonic's first change address */
        const val SECOND_CHANGE_0 = "tb1qmapywu8v8uztsqk4j8f5sarjzm2j9r04n8lauv"

        const val ALERT_TITLE = "Found Address"
    }

    @get:Rule
    val stayAwake = AndroidDeviceStayAwakeRule()

    private lateinit var device: UiDevice
    private lateinit var qr: QrScript
    private val node = FakeNodeControl()
    private val wallet by lazy { FullLaunchWalletRobot(device) }
    private var previousShowImeWithHardKeyboard: String? = null

    @Before
    fun prepareDevice() {
        device = fullLaunchDevice()
        qr = QrScript()

        // see UtxoLockFullLaunchTest: the onboarding robot must never see the soft keyboard
        previousShowImeWithHardKeyboard = device.executeShellCommand("settings get secure show_ime_with_hard_keyboard").trim()
        device.executeShellCommand("settings put secure show_ime_with_hard_keyboard 0")
    }

    @After
    fun restoreDevice() {
        qr.remove()
        when (val previous = previousShowImeWithHardKeyboard) {
            null, "null" -> device.executeShellCommand("settings delete secure show_ime_with_hard_keyboard")
            else -> device.executeShellCommand("settings put secure show_ime_with_hard_keyboard $previous")
        }
    }

    /**
     * Scanned from the second wallet, the first wallet's address names the first wallet, the second
     * wallet's own change address says it is this wallet's, and an address no wallet owns shows the
     * alert without either
     */
    @Test
    fun foundAddressAlertNamesTheWalletThatOwnsTheAddress() {
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
        wallet.waitForBalance(0L)
        val first = requireNotNull(wallet.selectedWalletName()) { "the first wallet has no name\n${device.hierarchy()}" }

        wallet.importFromAddWallet(SECOND_MNEMONIC, qr, sats = 0L)
        assertNotEquals("both wallets have the same name, so the alert can't tell them apart", first, wallet.selectedWalletName())

        scanFoundAddress(FIRST_RECEIVE_0)
        assertMessageContains("This is your address in “$first” (receive, index 0)")
        dismissAlert()

        scanFoundAddress(SECOND_CHANGE_0)
        assertMessageContains("This is an address in this wallet (change, index 0)")
        dismissAlert()

        // the alert waits for the lookup, so it shows without a note once the lookup found no owner
        scanFoundAddress(EXTERNAL_ADDRESS)
        assertFalse(
            "the alert named an owner for an address no wallet owns\n${device.hierarchy()}",
            device.hasObject(By.textContains("This is ")),
        )
        dismissAlert()
    }

    /** Opens the wallet screen's scanner and scans [address], which raises the Found Address alert */
    private fun scanFoundAddress(address: String) {
        // a tap while the last alert closes can land on nothing, so tap again until the sheet opens,
        // giving each tap time to open it so none opens it twice
        wallet.waitUntil("QR Code should open the wallet's scanner") {
            try {
                device.findObject(By.desc("QR Code"))?.click()
            } catch (_: StaleObjectException) {
            }

            val deadline = System.currentTimeMillis() + 3_000L
            while (!isScannerOpen() && System.currentTimeMillis() < deadline) Thread.sleep(100)
            isScannerOpen()
        }

        qr.show(listOf(address))
        device.waitFor(By.text(ALERT_TITLE), timeoutMs = 20_000L)
    }

    private fun assertMessageContains(expected: String) {
        assertTrue(
            "expected the alert to say $expected\n${device.hierarchy()}",
            device.wait(Until.hasObject(By.textContains(expected)), 5_000L),
        )
    }

    private fun dismissAlert() {
        device.waitFor(By.text("Cancel")).click()
        wallet.waitUntil("expected Cancel to close the alert") { !device.hasObject(By.text(ALERT_TITLE)) }
    }

    private fun isScannerOpen(): Boolean {
        var open = false
        InstrumentationRegistry.getInstrumentation().runOnMainSync {
            open = AppManager.getInstance().sheetState?.item == AppSheetState.Qr
        }

        return open
    }
}
