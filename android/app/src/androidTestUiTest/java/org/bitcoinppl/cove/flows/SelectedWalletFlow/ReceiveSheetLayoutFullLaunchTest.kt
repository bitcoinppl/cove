package org.bitcoinppl.cove.flows.SelectedWalletFlow

import android.content.ClipData
import android.content.ClipboardManager
import android.content.Context
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.filters.LargeTest
import androidx.test.platform.app.InstrumentationRegistry
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

        // no address reads like this, so only a copy that ran can replace it
        const val CLIPBOARD_SENTINEL = "receive sheet test: nothing copied yet"

        const val PAYMENT_SATS = 50_000L

        // the sheet polls the node for its address every 20 seconds while it is open
        const val PAYMENT_TIMEOUT_MS = 90_000L
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
     * and Copy Address must put the shown address on the clipboard and close the sheet. Paying
     * the shown address while the sheet is open must then show Payment Received, and every text
     * must still lay out and show whole
     *
     * The address takes no tap, so the clipboard and its new value after Create New Address are
     * what show it is the wallet's live address rather than a stale or cut-short copy. The sheet's
     * refresh failure label isn't reached: it shows only when saving a new address fails five
     * minutes after the address first showed, which no test seam can make happen
     */
    @Test
    fun receiveSheetFitsAndWorksAtLargestFontScale() {
        launchPastOnboarding()

        // the wallet's addresses are the same on every run, so a clipboard left by an earlier run
        // can already hold the one copied below; a value no address reads like must be replaced
        // by the copy. It goes on before Receive opens, since the system shows its clipboard
        // overlay over the screen for a few seconds after each write
        setClipboard(CLIPBOARD_SENTINEL)
        assertEquals("the clipboard didn't take the test's placeholder", CLIPBOARD_SENTINEL, clipboardText())

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

        checkPaidAddress()
    }

    /**
     * Opens Receive again, pays the address it shows on the fake node and waits for the open sheet
     * to mark it paid, then checks every text and the address and actions in that state
     */
    private fun checkPaidAddress() {
        screens.openReceive()
        val address = shownAddress()
        layout.awaitContentAtRest("the reopened $SHEET", By.text(address), By.text("Copy Address"))

        node.fund(address.replace(" ", ""), listOf(PAYMENT_SATS))
        device.waitFor(By.text("Payment Received"), timeoutMs = PAYMENT_TIMEOUT_MS)

        layout.scrollBackTo(By.text("Payment Received"), "$SHEET: Payment Received")
        val paid = layout.reachable(By.text("Payment Received"), "$SHEET: Payment Received", drawsWhole = "Payment Received")
        layout.assertWindowTextsDrawWhole("paid $SHEET", paid.visibleBounds)
        layout.assertWindowTextsShowWhole("paid $SHEET", device.waitFor(By.text("Payment Received")).visibleBounds)

        layout.scrollBackTo(By.text(address), "paid $SHEET: address")
        layout.reachable(By.text(address), "paid $SHEET: address", drawsWhole = address)
        layout.reachable(By.text("Copy Address"), "paid $SHEET: Copy Address", drawsWhole = "Copy Address", usable = true)
        layout.reachable(By.text("Create New Address"), "paid $SHEET: Create New Address", drawsWhole = "Create New Address", usable = true)
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

    private fun setClipboard(text: String) {
        val instrumentation = InstrumentationRegistry.getInstrumentation()
        instrumentation.runOnMainSync {
            val clipboard = instrumentation.targetContext.getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager
            clipboard.setPrimaryClip(ClipData.newPlainText("Receive sheet test", text))
        }
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
