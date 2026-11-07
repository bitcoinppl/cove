package org.bitcoinppl.cove.flows.SelectedWalletFlow

import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.filters.LargeTest
import androidx.test.uiautomator.By
import androidx.test.uiautomator.StaleObjectException
import androidx.test.uiautomator.UiDevice
import androidx.test.uiautomator.Until
import org.bitcoinppl.cove.test.AndroidDeviceStayAwakeRule
import org.bitcoinppl.cove.test.FakeNodeControl
import org.bitcoinppl.cove.test.FullLaunchLayoutChecks
import org.bitcoinppl.cove.test.FullLaunchOnboardingRobot
import org.bitcoinppl.cove.test.FullLaunchStartupRobot
import org.bitcoinppl.cove.test.FullLaunchWalletRobot
import org.bitcoinppl.cove.test.HardwareWalletScreens
import org.bitcoinppl.cove.test.SystemFontScale
import org.bitcoinppl.cove.test.clipboardText
import org.bitcoinppl.cove.test.fullLaunchDevice
import org.bitcoinppl.cove.test.launchFullApp
import org.bitcoinppl.cove.test.waitFor
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotEquals
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

/**
 * Opens Receive in the real lock-aware sheet at the largest system font scale and checks that the
 * address and both actions fit the sheet's window or scroll into it whole, and that the actions
 * still do their jobs (docs/testing.md)
 *
 * The sheet is its own window, which ignores a Compose test font scale, so this sets the system
 * font scale and restores it afterwards. Runs in the uiTest flavour; needs the fake node up and
 * forwarded, which `just android-instrumented-ci` does
 */
@LargeTest
@RunWith(AndroidJUnit4::class)
class ReceiveSheetLayoutFullLaunchTest {
    private companion object {
        const val SHEET = "receive sheet"

        // the wallet is on signet and Receive shows a native segwit address
        const val ADDRESS_PREFIX = "tb1q"
    }

    @get:Rule
    val stayAwake = AndroidDeviceStayAwakeRule()

    private lateinit var device: UiDevice
    private lateinit var wallet: FullLaunchWalletRobot
    private lateinit var screens: HardwareWalletScreens
    private lateinit var layout: FullLaunchLayoutChecks
    private lateinit var fontScale: SystemFontScale
    private val node = FakeNodeControl()
    private var previousShowImeWithHardKeyboard: String? = null

    @Before
    fun prepareDevice() {
        device = fullLaunchDevice()
        wallet = FullLaunchWalletRobot(device)
        screens = HardwareWalletScreens(device, wallet)
        layout = FullLaunchLayoutChecks(device, wallet)
        fontScale = SystemFontScale(device, wallet)

        // see UtxoLockFullLaunchTest: the onboarding robot must never see the soft keyboard
        previousShowImeWithHardKeyboard = device.executeShellCommand("settings get secure show_ime_with_hard_keyboard").trim()
        device.executeShellCommand("settings put secure show_ime_with_hard_keyboard 0")
        fontScale.save()
    }

    @After
    fun restoreDevice() {
        fontScale.restore()

        when (val previous = previousShowImeWithHardKeyboard) {
            null, "null" -> device.executeShellCommand("settings delete secure show_ime_with_hard_keyboard")
            else -> device.executeShellCommand("settings put secure show_ime_with_hard_keyboard $previous")
        }
    }

    /**
     * Every text in the sheet must lay out and show whole, the address and both actions must
     * scroll into reach unclipped and unsqueezed, Create New Address must show another address,
     * and Copy Address must put the shown address on the clipboard and close the sheet
     *
     * The address takes no tap, so the clipboard and its new value after Create New Address are
     * what show it is the wallet's live address rather than a stale or cut-short copy
     */
    @Test
    fun receiveSheetFitsAndWorksAtLargestFontScale() {
        launchPastOnboarding()

        // onboarding runs at the default scale, since only the receive sheet is under test
        fontScale.setLargestFontScaleAndWaitForRecreation()
        device.waitFor(By.res("selectedWallet.balance"), timeoutMs = 30_000L)
        screens.openReceive()

        // the sheet slides in, and Copy Address stays disabled until the address loads
        val first = shownAddress()
        layout.awaitContentAtRest("the $SHEET", By.text(first), By.text("Copy Address"))
        layout.scrollBackTo(By.text(first), "$SHEET: address")
        val addressNode = layout.reachable(By.text(first), "$SHEET: address", drawsWhole = first)

        // the address is in the sheet's window alone, so it marks that window for every text in it
        layout.assertWindowTextsDrawWhole(SHEET, addressNode.visibleBounds)
        layout.assertWindowTextsShowWhole(SHEET, device.waitFor(By.text(first)).visibleBounds)

        layout.reachable(By.text("Copy Address"), "$SHEET: Copy Address", drawsWhole = "Copy Address", usable = true)
        layout
            .reachable(By.text("Create New Address"), "$SHEET: Create New Address", drawsWhole = "Create New Address", usable = true)
            .click()

        // the new address must show whole too, back at the top of the sheet
        var second = first
        wallet.waitUntil("Create New Address kept the address $first", timeoutMs = 30_000L) {
            second = shownAddress()
            second != first
        }

        layout.scrollBackTo(By.text(second), "$SHEET: new address")
        layout.reachable(By.text(second), "$SHEET: new address", drawsWhole = second)

        layout.reachable(By.text("Copy Address"), "$SHEET: Copy Address", drawsWhole = "Copy Address", usable = true).click()
        assertTrue("Copy Address didn't close the $SHEET", device.wait(Until.gone(By.text("Copy Address")), 15_000L))

        // the sheet shows the address in groups of four, and copies it without the spaces
        val copied = clipboardText().trim()
        assertEquals("Copy Address copied something other than the shown address", second.replace(" ", ""), copied)
        assertNotEquals("Copy Address copied the address Create New Address replaced", first.replace(" ", ""), copied)
    }

    /** Starts a fresh app on the reset fake node and imports a software wallet to finish onboarding */
    private fun launchPastOnboarding() {
        // a reset node gives the wallet no history, so its receive address has no payment and
        // copies without asking first
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

    /** The address the sheet shows, once it has loaded one */
    private fun shownAddress(): String {
        var address: String? = null

        wallet.waitUntil("the $SHEET never showed an address", timeoutMs = 30_000L) {
            // a recomposition can replace the node between finding and reading it
            address =
                try {
                    device.findObject(By.textStartsWith(ADDRESS_PREFIX))?.text
                } catch (_: StaleObjectException) {
                    null
                }

            address != null
        }

        return checkNotNull(address)
    }
}
