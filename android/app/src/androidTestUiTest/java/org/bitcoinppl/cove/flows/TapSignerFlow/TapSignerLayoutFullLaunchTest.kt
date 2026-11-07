package org.bitcoinppl.cove.flows.TapSignerFlow

import android.graphics.Rect
import android.system.Os
import android.view.View
import android.view.WindowInsets
import android.view.WindowInsetsAnimation
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.filters.LargeTest
import androidx.test.platform.app.InstrumentationRegistry
import androidx.test.runner.lifecycle.ActivityLifecycleMonitorRegistry
import androidx.test.runner.lifecycle.Stage
import androidx.test.uiautomator.By
import androidx.test.uiautomator.BySelector
import androidx.test.uiautomator.StaleObjectException
import androidx.test.uiautomator.UiDevice
import androidx.test.uiautomator.Until
import org.bitcoinppl.cove.test.AndroidDeviceStayAwakeRule
import org.bitcoinppl.cove.test.FAKE_NODE_ESPLORA_URL
import org.bitcoinppl.cove.test.FakeNodeControl
import org.bitcoinppl.cove.test.FullLaunchLayoutChecks
import org.bitcoinppl.cove.test.FullLaunchLayoutChecks.Companion.MAX_NUDGES
import org.bitcoinppl.cove.test.FullLaunchLayoutChecks.ControlBounds
import org.bitcoinppl.cove.test.FullLaunchOnboardingRobot
import org.bitcoinppl.cove.test.FullLaunchStartupRobot
import org.bitcoinppl.cove.test.FullLaunchWalletRobot
import org.bitcoinppl.cove.test.HardwareSignerControl
import org.bitcoinppl.cove.test.HardwareWalletScreens
import org.bitcoinppl.cove.test.LARGEST_SYSTEM_FONT_SCALE
import org.bitcoinppl.cove.test.SystemFontScale
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
import kotlin.math.abs

/**
 * Walks every TAPSIGNER setup and import screen, and the CVC screen signing opens, at the largest
 * system font scale and checks that each screen's text and actions fit the display or scroll into
 * it, through the debug TAPSIGNER seam and an emulated card (docs/testing.md)
 *
 * The TAPSIGNER sheet is its own window, which ignores a Compose test font scale, so this sets the
 * system font scale and restores it afterwards. Each node's full bounds and text layout come from
 * the Compose semantics tree, so the checks catch text and actions pushed off screen, cut off by
 * their container or squeezed, and text that lays out ellipsized or cut short inside its own bounds.
 * Runs in the uiTest flavour; needs the hardware signer host, and the fake node for the signing
 * test, up and forwarded, which `just android-instrumented-ci` does
 */
@LargeTest
@RunWith(AndroidJUnit4::class)
class TapSignerLayoutFullLaunchTest {
    private companion object {
        const val NEW_CVC = "246810"
        const val FUNDED_SATS = 100_000L

        // where signing's CVC message differs from import's
        const val SIGNING_MESSAGE_START = "For security purposes, you must enter your TAPSIGNER PIN before you can sign"

        // drops the card's session once the operation has started on it, so the app offers to
        // continue instead of failing outright
        const val EXCHANGES_BEFORE_DROP = 3

    }

    @get:Rule
    val stayAwake = AndroidDeviceStayAwakeRule()

    private lateinit var device: UiDevice
    private lateinit var wallet: FullLaunchWalletRobot
    private lateinit var screens: HardwareWalletScreens
    private lateinit var layout: FullLaunchLayoutChecks
    private lateinit var fontScale: SystemFontScale
    private val node = FakeNodeControl()
    private val signer = HardwareSignerControl()
    private var previousShowImeWithHardKeyboard: String? = null
    private var imeAnimations: ImeAnimationProbe? = null

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
     * A funded card's Sign using TAPSIGNER opens the CVC screen with signing's own message
     *
     * That screen shows before the app contacts the card, so signing failing on the card (#965)
     * doesn't stop the check; the test cancels instead of submitting the CVC. The card is imported
     * and funded at the default scale, since only the signing screen is under test, and the scale
     * changes on the wallet screen, which the activity the change recreates opens again
     */
    @Test
    fun tapSignerSigningCvcScreenFitsAtLargestFontScale() {
        signer.requireHealthy()

        // debug builds read this on every fee fetch, so the draft prices its fee from the fake node
        Os.setenv("COVE_FEE_URL", "$FAKE_NODE_ESPLORA_URL/api/v1/fees/recommended", true)
        launchPastOnboarding(needsSetup = false)

        screens.scanTapSignerFromAddWallet("Import TAPSIGNER?")
        screens.enterTapSignerCvc(TAPSIGNER_CVC)
        device.waitFor(By.text("Import Complete"), timeoutMs = 30_000L)
        device.waitFor(By.text("Continue")).click()
        device.waitFor(By.res("selectedWallet.balance"), timeoutMs = 30_000L)

        node.fund(screens.copyReceiveAddress(), listOf(FUNDED_SATS))
        screens.refreshUntilBalance(FUNDED_SATS)

        fontScale.setLargestFontScaleAndWaitForRecreation()
        wallet.waitForBalance(FUNDED_SATS)

        // onboarding is done, so the soft keyboard can show beside the emulator's hardware one,
        // as it does on a phone, which is the keyboard the address field must stay above
        device.executeShellCommand("settings put secure show_ime_with_hard_keyboard 1")

        // the probe goes on before Send, so it hears every keyboard animation the send screen starts
        imeAnimations = ImeAnimationProbe.attach()

        // the send screen opens with the amount focused and the keyboard up, and at this scale
        // every field must still scroll into reach above the keyboard: the address field below
        // the amount, and Next at the end of the form, which only stops under the keyboard when
        // the scroll area runs on beneath it
        device.waitFor(By.text("Send")).click()
        device.waitFor(By.res("sendFlow.amount"))
        scrollAboveKeyboard(By.res("sendFlow.address"), "send: address field")
        wallet.enterAddress()
        wallet.tapMax()

        // with an address in, Max clears the focus and closes the keyboard, so the amount field
        // opens it again, with the toolbar the form must also stop above. The tag sits on the box
        // around the text field, so the focus shows on a node inside it
        device.scrollTo(By.res("sendFlow.amount")).click()
        device.waitFor(By.res("sendFlow.amount").hasDescendant(By.focused(true)))
        scrollAboveKeyboard(By.res("sendFlow.next"), "send: Next", toolbar = By.res("sendFlow.keyboardToolbar"))

        // at this scale the hardware send screen ends its first screenful at its Sign Transaction
        // heading, with the signing options below the fold
        wallet.clickNext(expecting = By.text("Sign Transaction"))
        layout.reachable(By.text("Sign using TAPSIGNER"), "hardware send: \"Sign using TAPSIGNER\"", usable = true).click()

        val title = "Enter TAPSIGNER CVC"
        assertCvcScreenFits(title, "Cancel")
        device.waitFor(By.textStartsWith(SIGNING_MESSAGE_START))

        // the submit action stays disabled until the field holds a whole CVC, which is typed but
        // never submitted
        wallet.typeInto(By.clazz("android.widget.EditText"), TAPSIGNER_CVC) {
            device.hasObject(By.text("Digits: ${TAPSIGNER_CVC.length}"))
        }

        wallet.dismissKeyboard()
        layout.reachable(By.text("Submit").enabled(true), "$title: Submit", drawsWhole = "Submit", usable = true)

        layout.reachable(By.text("Cancel"), "$title: Cancel", usable = true).click()
        assertTrue("Cancel didn't close the signing CVC screen", device.wait(Until.gone(By.text(title)), 15_000L))
        device.scrollTo(By.text("Sign using TAPSIGNER"), timeoutMs = 15_000L)
    }

    /**
     * Finishes onboarding at the default scale, since Add Wallet needs a wallet and only the
     * TAPSIGNER screens are under test, then sets the largest scale and scans the card
     */
    private fun openTapSignerAtLargestFontScale(
        needsSetup: Boolean,
        alert: String,
    ) {
        launchPastOnboarding(needsSetup)

        device.executeShellCommand("settings put system font_scale $LARGEST_SYSTEM_FONT_SCALE")
        device.waitForIdle()
        screens.scanTapSignerFromAddWallet(alert)
    }

    /** Starts the card and a fresh app on the fake node, and imports a software wallet to finish onboarding */
    private fun launchPastOnboarding(needsSetup: Boolean) {
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
    }

    // MARK: setup

    private fun setUpCardThroughEveryScreen() {
        assertScreenFits("choose chain code", "Cancel", "Setup Chain Code", "Automatic Setup", "Advanced Setup")
        device.findObject(By.text("Advanced Setup")).click()

        // the advanced screen is the one with Back where the choose screen has Cancel
        device.waitFor(By.text("Back"))
        assertScreenFits("advanced setup", "Back", "Advanced Setup")
        layout.reachable(By.clazz("android.widget.EditText"), "advanced setup: chain code field")
        wallet.typeInto(By.clazz("android.widget.EditText"), randomChainCode())
        wallet.dismissKeyboard()
        layout.reachable(By.text("Continue").enabled(true), "advanced setup: Continue", drawsWhole = "Continue", usable = true).click()

        enterCvc(TAPSIGNER_CVC, "Enter Factory CVC", "Back", "Continue", next = "Create New CVC")
        enterCvc(NEW_CVC, "Create New CVC", "Back", "Continue", next = "Confirm New CVC")

        // the card leaves the field once setup has started on it
        signer.dropNextTapSignerSession(afterExchanges = EXCHANGES_BEFORE_DROP)
        enterCvc(NEW_CVC, "Confirm New CVC", "Back", "Confirm and continue", next = "Continue Setup")
        assertScreenFits("setup retry", "Cancel", "Setup in Progress", "Continue Setup")

        // each scan can stop at another stage, which offers to continue again
        repeat(3) {
            if (device.hasObject(By.text("Setup Complete"))) return@repeat

            layout.reachable(By.text("Continue Setup").enabled(true), "Continue Setup", usable = true).click()
            device.wait(Until.hasObject(By.text("Setup Complete")), 30_000L)
        }

        assertScreenFits("setup success", "Cancel", "Setup Complete", "Download Backup", "Continue")
        layout.reachable(By.text("Continue"), "setup success: Continue", usable = true).click()
        device.waitFor(By.res("selectedWallet.balance"), timeoutMs = 30_000L)
    }

    // MARK: import

    private fun importCardThroughEveryScreen() {
        signer.dropNextTapSignerSession(afterExchanges = EXCHANGES_BEFORE_DROP)
        enterCvc(TAPSIGNER_CVC, "Enter TAPSIGNER CVC", "Cancel", "Submit", next = "Continue Import")
        assertScreenFits("import retry", "Cancel", "Import needs to continue", "Continue Import")
        layout.reachable(By.text("Continue Import"), "Continue Import", usable = true).click()

        device.waitFor(By.text("Import Complete"), timeoutMs = 30_000L)
        assertScreenFits("import success", "Cancel", "Import Complete", "Continue")
        layout.reachable(By.text("Continue"), "import success: Continue", usable = true).click()
        device.waitFor(By.res("selectedWallet.balance"), timeoutMs = 30_000L)
    }

    // MARK: CVC screens

    /**
     * Checks the CVC screen fits, then enters the CVC, submits it and waits for the screen after it,
     * which shows the text [next]
     *
     * The title alone can't mark either screen: the field scrolls a tall message's title above the
     * top. So the CVC goes in once the field shows, the check above having said which screen it is,
     * and the next screen counts as open only once [next] shows, dragging back to the top for it if
     * that screen opened at its field. A CVC screen that stays open never shows [next], so it fails
     */
    private fun enterCvc(
        cvc: String,
        title: String,
        back: String,
        submit: String,
        next: String,
    ) {
        assertCvcScreenFits(title, back)

        screens.enterTapSignerCvc(cvc, title, submit, opened = By.clazz("android.widget.EditText")) {
            layout.reachable(it, "$title: $submit", drawsWhole = submit, usable = true)
        }

        if (!device.wait(Until.hasObject(By.text(next)), 30_000L)) {
            layout.scrollBackTo(By.text(next), "after $title accepted the CVC, \"$next\"")
        }
    }

    /** Checks the CVC screen titled [title] fits, before any CVC is entered */
    private fun assertCvcScreenFits(
        title: String,
        back: String,
    ) {
        // the screen requests no focus and opens at its top, but a tall message can leave the title
        // or the field as the only one of the two on screen, so either marks it open
        wallet.waitUntil("the $title screen never showed", timeoutMs = 15_000L) {
            device.hasObject(By.text(title)) || device.hasObject(By.text("CVC"))
        }

        // the sheet slides in, and a node read while its content moves goes stale, so the checks
        // start once the content holds still, then scroll back to the top in case it opened lower,
        // and scroll to the field explicitly at the end
        layout.awaitContentAtRest("the $title screen", By.text(title), By.text("CVC"))
        layout.scrollBackTo(By.text(back), "$title: \"$back\"")
        assertScreenFits(title, back, title)
        layout.reachable(By.clazz("android.widget.EditText"), "$title: CVC field")
    }

    // MARK: layout checks

    /**
     * Checks the back action, the title and then each action in order from the top, scrolling down
     * to reach the later ones; each must draw whole, and each action must take a click
     *
     * Once the title shows, every other text in its window, such as the descriptions, must lay
     * out whole too
     */
    private fun assertScreenFits(
        screen: String,
        back: String,
        title: String,
        vararg actions: String,
    ) {
        layout.reachable(By.text(back), "$screen: \"$back\"", drawsWhole = back, usable = true)

        val titleNode = layout.reachable(By.text(title), "$screen: \"$title\"", drawsWhole = title)
        layout.assertWindowTextsDrawWhole(screen, titleNode.visibleBounds)
        layout.assertWindowTextsShowWhole(screen, titleNode.visibleBounds)

        actions.forEach { layout.reachable(By.text(it), "$screen: \"$it\"", drawsWhole = it, usable = true) }
    }

    /**
     * Waits for the keyboard to come to rest, then drags the scroll area above it until the control
     * matching [selector] shows whole there, and fails if it never does
     *
     * With [toolbar], the space ends at the top of that bar on the keyboard, or at the keyboard's
     * top if that is higher, so a control behind the bar fails too. Every measurement is taken once
     * the window's keyboard animations have ended, see [settledPlacement], so a keyboard still
     * sliding open can't pass or fail the check. The drags stay in that space, where a drag on the
     * keys would type instead of scroll, and each moves the content only a short way, so a control
     * at the end of a long form gets as many drags as [FullLaunchLayoutChecks.scrollBackTo] gives one at its top
     */
    private fun scrollAboveKeyboard(
        selector: BySelector,
        name: String,
        toolbar: BySelector? = null,
    ) {
        val cover = if (toolbar == null) "keyboard" else "keyboard and its toolbar"
        val drags = MAX_NUDGES * 3
        var last: String? = null

        repeat(drags) {
            val placement = settledPlacement(selector, toolbar, "expected the $cover to come to rest before $name")
            val bounds = placement.control
            if (bounds != null && bounds.whole.bottom <= placement.coverTop && bounds.shown.contains(bounds.whole)) return

            val viewport = layout.largestScrollArea(name)
            last = "$bounds with the $cover from ${placement.coverTop}, dragging in $viewport"

            layout.nudge(Rect(viewport.left, viewport.top, viewport.right, minOf(viewport.bottom, placement.coverTop)), towardsTop = true)
        }

        error("$name never showed whole above the $cover after $drags drags: ${last ?: "not found"}")
    }

    /**
     * Where the keyboard, the toolbar on it if any, and a control lie at one moment, and how many
     * keyboard animations had ended by then
     */
    private data class KeyboardPlacement(
        val keyboardTop: Int,
        val toolbar: Rect?,
        val control: ControlBounds?,
        val animationsEnded: Int,
    ) {
        /** The top of whatever covers the screen from below, the toolbar or the keyboard */
        val coverTop: Int get() = minOf(keyboardTop, toolbar?.top ?: keyboardTop)
    }

    /**
     * Reads the keyboard, the [toolbar] and the control matching [selector] until two reads in a row
     * agree and no keyboard animation started or ended between them
     *
     * The window takes the keyboard's final insets as its animation starts, so [keyboardTop] already
     * holds its final value while the screen still slides with the keyboard. A read counts only
     * once [ImeAnimationProbe] has heard every animation it saw start end, with the keyboard open,
     * and with the toolbar, if one is asked for, laid out with its bottom on the keyboard's final
     * top, within [KEYBOARD_EDGE_ROUNDING_PX], and with no node going stale while it was read. The
     * agreeing second read lets the frame that applies the final insets land first
     */
    private fun settledPlacement(
        selector: BySelector,
        toolbar: BySelector?,
        message: String,
    ): KeyboardPlacement {
        var previous: KeyboardPlacement? = null
        var settled: KeyboardPlacement? = null

        wallet.waitUntil(message) {
            val read = readPlacement(selector, toolbar)
            if (read != null && read == previous) settled = read
            previous = read
            settled != null
        }

        return settled ?: error(message)
    }

    private fun readPlacement(
        selector: BySelector,
        toolbar: BySelector?,
    ): KeyboardPlacement? {
        val animations = imeAnimations ?: error("the keyboard animation probe must go on before the screen opens the keyboard")
        val ended = animations.endedIfIdle() ?: return null
        val keyboard = layout.keyboardTop() ?: return null

        // a node replaced between finding and reading it makes this read count as not yet at rest
        return try {
            val bar =
                toolbar?.let {
                    device
                        .findObject(it)
                        ?.visibleBounds
                        ?.takeIf { bounds -> !bounds.isEmpty && abs(bounds.bottom - keyboard) <= KEYBOARD_EDGE_ROUNDING_PX }
                        ?: return null
                }

            KeyboardPlacement(keyboard, bar, device.findObject(selector)?.visibleBounds?.let(layout::controlBounds), ended)
        } catch (_: StaleObjectException) {
            null
        }
    }

    private fun randomChainCode(): String = ByteArray(32).also(SecureRandom()::nextBytes).joinToString("") { "%02x".format(it) }
}

/** How far, in pixels, rounding can put the toolbar's bottom from the keyboard's top */
private const val KEYBOARD_EDGE_ROUNDING_PX = 2

/**
 * Hears the keyboard's inset animations on the resumed activity's window
 *
 * It sits on the activity's content view, which holds no callback of its own, and passes every
 * event on to the views below, so the app's own insets handling sees them unchanged. It counts
 * only the animations it saw start, so it must go on before the screen opens the keyboard
 */
private class ImeAnimationProbe private constructor(
    private val host: View,
) : WindowInsetsAnimation.Callback(DISPATCH_MODE_CONTINUE_ON_SUBTREE) {
    private var running = 0
    private var ended = 0

    override fun onStart(
        animation: WindowInsetsAnimation,
        bounds: WindowInsetsAnimation.Bounds,
    ): WindowInsetsAnimation.Bounds {
        if (animation.isIme()) running++
        return bounds
    }

    override fun onProgress(
        insets: WindowInsets,
        runningAnimations: MutableList<WindowInsetsAnimation>,
    ): WindowInsets = insets

    override fun onEnd(animation: WindowInsetsAnimation) {
        if (!animation.isIme()) return

        running = maxOf(0, running - 1)
        ended++
    }

    /**
     * How many keyboard animations have ended, or null while one is still running, and fails if
     * the resumed activity is no longer the one the probe is on
     */
    fun endedIfIdle(): Int? {
        var result: Int? = null
        var onResumedWindow = false

        InstrumentationRegistry.getInstrumentation().runOnMainSync {
            onResumedWindow = resumedContentView() === host
            if (running == 0) result = ended
        }

        check(onResumedWindow) { "the app replaced the window the keyboard animation probe is on" }
        return result
    }

    private fun WindowInsetsAnimation.isIme(): Boolean = typeMask and WindowInsets.Type.ime() != 0

    companion object {
        /** Puts a probe on the resumed activity's content view */
        fun attach(): ImeAnimationProbe {
            var probe: ImeAnimationProbe? = null

            InstrumentationRegistry.getInstrumentation().runOnMainSync {
                val host = resumedContentView() ?: return@runOnMainSync
                probe = ImeAnimationProbe(host).also(host::setWindowInsetsAnimationCallback)
            }

            return probe ?: error("expected a resumed activity to put the keyboard animation probe on")
        }

        private fun resumedContentView(): View? =
            ActivityLifecycleMonitorRegistry
                .getInstance()
                .getActivitiesInStage(Stage.RESUMED)
                .firstOrNull()
                ?.findViewById(android.R.id.content)
    }
}
