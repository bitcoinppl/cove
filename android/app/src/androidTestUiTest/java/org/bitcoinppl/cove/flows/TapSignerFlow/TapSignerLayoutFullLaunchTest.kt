package org.bitcoinppl.cove.flows.TapSignerFlow

import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.filters.LargeTest
import androidx.test.platform.app.InstrumentationRegistry
import androidx.test.uiautomator.By
import androidx.test.uiautomator.BySelector
import androidx.test.uiautomator.UiDevice
import androidx.test.uiautomator.Until
import org.bitcoinppl.cove.test.AndroidDeviceStayAwakeRule
import org.bitcoinppl.cove.test.FakeNodeControl
import org.bitcoinppl.cove.test.FullLaunchOnboardingRobot
import org.bitcoinppl.cove.test.FullLaunchStartupRobot
import org.bitcoinppl.cove.test.FullLaunchWalletRobot
import org.bitcoinppl.cove.test.HardwareSignerControl
import org.bitcoinppl.cove.test.HardwareWalletScreens
import org.bitcoinppl.cove.test.LARGEST_SYSTEM_FONT_SCALE
import org.bitcoinppl.cove.test.TAPSIGNER_CVC
import org.bitcoinppl.cove.test.fullLaunchDevice
import org.bitcoinppl.cove.test.launchFullApp
import org.bitcoinppl.cove.test.scrollTo
import org.bitcoinppl.cove.test.waitFor
import org.junit.After
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import java.security.SecureRandom

/**
 * Walks every TAPSIGNER setup and import screen at the largest system font scale and checks that
 * each screen's text and actions fit the display or scroll into it, through the debug TAPSIGNER seam
 * and an emulated card (docs/testing.md)
 *
 * The TAPSIGNER sheet is its own window, which ignores a Compose test font scale, so this sets the
 * system font scale and restores it afterwards. UiAutomator sees a node's visible bounds, so the
 * checks catch text and actions pushed off screen or squeezed, not an ellipsis inside a label.
 * Runs in the uiTest flavour; needs the hardware signer host up and forwarded, which
 * `just android-instrumented-ci` does
 */
@LargeTest
@RunWith(AndroidJUnit4::class)
class TapSignerLayoutFullLaunchTest {
    private companion object {
        const val NEW_CVC = "246810"

        // drops the card's session once the operation has started on it, so the app offers to
        // continue instead of failing outright
        const val EXCHANGES_BEFORE_DROP = 3

        // well under a line of text at 2x, so only a squeezed or clipped node falls below it
        const val MIN_NODE_HEIGHT_DP = 24
    }

    @get:Rule
    val stayAwake = AndroidDeviceStayAwakeRule()

    private lateinit var device: UiDevice
    private lateinit var wallet: FullLaunchWalletRobot
    private lateinit var screens: HardwareWalletScreens
    private val node = FakeNodeControl()
    private val signer = HardwareSignerControl()
    private var previousShowImeWithHardKeyboard: String? = null
    private var previousFontScale: String? = null

    @Before
    fun prepareDevice() {
        device = fullLaunchDevice()
        wallet = FullLaunchWalletRobot(device)
        screens = HardwareWalletScreens(device, wallet)

        // see UtxoLockFullLaunchTest: the onboarding robot must never see the soft keyboard
        previousShowImeWithHardKeyboard = device.executeShellCommand("settings get secure show_ime_with_hard_keyboard").trim()
        device.executeShellCommand("settings put secure show_ime_with_hard_keyboard 0")
        previousFontScale = device.executeShellCommand("settings get system font_scale").trim()
    }

    @After
    fun restoreDevice() {
        when (val previous = previousFontScale) {
            null, "null" -> device.executeShellCommand("settings delete system font_scale")
            else -> device.executeShellCommand("settings put system font_scale $previous")
        }

        when (val previous = previousShowImeWithHardKeyboard) {
            null, "null" -> device.executeShellCommand("settings delete secure show_ime_with_hard_keyboard")
            else -> device.executeShellCommand("settings put secure show_ime_with_hard_keyboard $previous")
        }
    }

    /** A new card walks choose, advanced, the three PIN screens, setup retry and setup success */
    @Test
    fun tapSignerSetupScreensFitAtLargestFontScale() {
        openTapSignerAtLargestFontScale(needsSetup = true, alert = "Setup TAPSIGNER?")
        setUpCardThroughEveryScreen()
    }

    /**
     * A card that already has a key walks the CVC screen, import retry and import success
     *
     * The emulator's seed fixes both its card identity and the root certificate the debug build
     * trusts, so a set-up card is the same card the setup test made and would open its wallet; each
     * test starts from reset app data instead
     */
    @Test
    fun tapSignerImportScreensFitAtLargestFontScale() {
        openTapSignerAtLargestFontScale(needsSetup = false, alert = "Import TAPSIGNER?")
        importCardThroughEveryScreen()
    }

    /**
     * Finishes onboarding at the default scale, since Add Wallet needs a wallet and only the
     * TAPSIGNER screens are under test, then sets the largest scale and scans the card
     */
    private fun openTapSignerAtLargestFontScale(
        needsSetup: Boolean,
        alert: String,
    ) {
        node.reset()
        signer.startTapSigner(needsSetup)
        HardwareSignerControl.useTapSignerBridge()

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

        device.executeShellCommand("settings put system font_scale $LARGEST_SYSTEM_FONT_SCALE")
        device.waitForIdle()
        screens.scanTapSignerFromAddWallet(alert)
    }

    // MARK: setup

    private fun setUpCardThroughEveryScreen() {
        assertScreenFits("choose chain code", "Cancel", "Setup Chain Code", "Automatic Setup", "Advanced Setup")
        device.findObject(By.text("Advanced Setup")).click()

        // the advanced screen is the one with Back where the choose screen has Cancel
        device.waitFor(By.text("Back"))
        assertScreenFits("advanced setup", "Back", "Advanced Setup")
        reachable(By.clazz("android.widget.EditText"), "advanced setup: chain code field")
        wallet.typeInto(By.clazz("android.widget.EditText"), randomChainCode())
        wallet.dismissKeyboard()
        reachable(By.text("Continue").enabled(true), "advanced setup: Continue").click()

        enterCvc(TAPSIGNER_CVC, "Enter Factory CVC", "Back", "Continue")
        enterCvc(NEW_CVC, "Create New CVC", "Back", "Continue")

        // the card leaves the field once setup has started on it
        signer.dropNextTapSignerSession(afterExchanges = EXCHANGES_BEFORE_DROP)
        enterCvc(NEW_CVC, "Confirm New CVC", "Back", "Confirm and continue")

        device.waitFor(By.text("Continue Setup"), timeoutMs = 30_000L)
        assertScreenFits("setup retry", "Cancel", "Setup in Progress", "Continue Setup")

        // each scan can stop at another stage, which offers to continue again
        repeat(3) {
            if (device.hasObject(By.text("Setup Complete"))) return@repeat

            reachable(By.text("Continue Setup").enabled(true), "Continue Setup").click()
            device.wait(Until.hasObject(By.text("Setup Complete")), 30_000L)
        }

        assertScreenFits("setup success", "Cancel", "Setup Complete", "Download Backup", "Continue")
        reachable(By.text("Continue"), "setup success: Continue").click()
        device.waitFor(By.res("selectedWallet.balance"), timeoutMs = 30_000L)
    }

    // MARK: import

    private fun importCardThroughEveryScreen() {
        signer.dropNextTapSignerSession(afterExchanges = EXCHANGES_BEFORE_DROP)
        enterCvc(TAPSIGNER_CVC, "Enter TAPSIGNER CVC", "Cancel", "Submit")

        device.waitFor(By.text("Continue Import"), timeoutMs = 30_000L)
        assertScreenFits("import retry", "Cancel", "Import needs to continue", "Continue Import")
        reachable(By.text("Continue Import"), "Continue Import").click()

        device.waitFor(By.text("Import Complete"), timeoutMs = 30_000L)
        assertScreenFits("import success", "Cancel", "Import Complete", "Continue")
        reachable(By.text("Continue"), "import success: Continue").click()
        device.waitFor(By.res("selectedWallet.balance"), timeoutMs = 30_000L)
    }

    // MARK: CVC screens

    /** Checks the CVC screen fits, then enters the CVC and submits it */
    private fun enterCvc(
        cvc: String,
        title: String,
        back: String,
        submit: String,
    ) {
        device.waitFor(By.text(title), timeoutMs = 15_000L)
        assertScreenFits(title, back, title)
        reachable(By.clazz("android.widget.EditText"), "$title: CVC field")

        screens.enterTapSignerCvc(cvc, title, submit)
        assertTrue(
            "the $title screen didn't accept the CVC",
            device.wait(Until.gone(By.text(title)), 15_000L),
        )
    }

    // MARK: layout checks

    /** Checks each text in order from the top, scrolling down to reach the later ones */
    private fun assertScreenFits(
        screen: String,
        vararg texts: String,
    ) {
        texts.forEach { reachable(By.text(it), "$screen: \"$it\"") }
    }

    /**
     * Scrolls until the node lies above the gesture strip, then checks it sits wholly inside the
     * display and isn't squeezed
     */
    private fun reachable(
        selector: BySelector,
        name: String,
    ) = device.scrollTo(selector, timeoutMs = 15_000L).also { node ->
        val bounds = node.visibleBounds
        val density = InstrumentationRegistry.getInstrumentation().targetContext.resources.displayMetrics.density

        assertTrue(
            "$name is $bounds, outside the ${device.displayWidth}x${device.displayHeight} display",
            bounds.left >= 0 && bounds.top >= 0 && bounds.right <= device.displayWidth && bounds.bottom <= device.displayHeight,
        )
        assertTrue(
            "$name is squeezed to $bounds, under ${MIN_NODE_HEIGHT_DP}dp tall",
            bounds.height() >= MIN_NODE_HEIGHT_DP * density,
        )
    }

    private fun randomChainCode(): String = ByteArray(32).also(SecureRandom()::nextBytes).joinToString("") { "%02x".format(it) }
}
