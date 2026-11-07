package org.bitcoinppl.cove.flows.TapSignerFlow

import android.graphics.Rect
import android.system.Os
import android.view.View
import android.view.ViewGroup
import android.view.WindowInsets
import android.view.WindowInsetsAnimation
import android.view.inspector.WindowInspector
import androidx.compose.ui.geometry.Rect as ComposeRect
import androidx.compose.ui.node.RootForTest
import androidx.compose.ui.semantics.SemanticsActions
import androidx.compose.ui.semantics.SemanticsNode
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.semantics.getAllSemanticsNodes
import androidx.compose.ui.semantics.getOrNull
import androidx.compose.ui.text.TextLayoutResult
import androidx.compose.ui.unit.toSize
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.filters.LargeTest
import androidx.test.platform.app.InstrumentationRegistry
import androidx.test.runner.lifecycle.ActivityLifecycleMonitorRegistry
import androidx.test.runner.lifecycle.Stage
import androidx.test.uiautomator.By
import androidx.test.uiautomator.BySelector
import androidx.test.uiautomator.StaleObjectException
import androidx.test.uiautomator.UiDevice
import androidx.test.uiautomator.UiObject2
import androidx.test.uiautomator.Until
import org.bitcoinppl.cove.test.AndroidDeviceStayAwakeRule
import org.bitcoinppl.cove.test.FAKE_NODE_ESPLORA_URL
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
import kotlin.math.abs
import kotlin.math.roundToInt

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

        // well under a line of text at 2x, so only a squeezed or clipped node falls below it
        const val MIN_NODE_HEIGHT_DP = 24

        // how far one drag moves a node cut off at the top or bottom, and how many it gets
        const val NUDGE_DP = 96
        const val MAX_NUDGES = 6

        // Compose positions are floats that the accessibility bounds round
        const val CLIP_TOLERANCE_PX = 1

        // a text's box is its widest line rounded up to a whole pixel
        const val LINE_WIDTH_TOLERANCE_PX = 1f

        // enough of a text to tell which one a failure names
        const val TEXT_IN_MESSAGE = 40

        // taps in gesture navigation's bottom strip go to the system, not the app
        const val GESTURE_STRIP_DP = 48
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
    private var imeAnimations: ImeAnimationProbe? = null

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

        setLargestFontScaleAndWaitForRecreation()
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
        reachable(By.text("Sign using TAPSIGNER"), "hardware send: \"Sign using TAPSIGNER\"", usable = true).click()

        val title = "Enter TAPSIGNER CVC"
        assertCvcScreenFits(title, "Cancel")
        device.waitFor(By.textStartsWith(SIGNING_MESSAGE_START))

        // the submit action stays disabled until the field holds a whole CVC, which is typed but
        // never submitted
        wallet.typeInto(By.clazz("android.widget.EditText"), TAPSIGNER_CVC) {
            device.hasObject(By.text("Digits: ${TAPSIGNER_CVC.length}"))
        }

        wallet.dismissKeyboard()
        reachable(By.text("Submit").enabled(true), "$title: Submit", drawsWhole = "Submit", usable = true)

        reachable(By.text("Cancel"), "$title: Cancel", usable = true).click()
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
        reachable(By.clazz("android.widget.EditText"), "advanced setup: chain code field")
        wallet.typeInto(By.clazz("android.widget.EditText"), randomChainCode())
        wallet.dismissKeyboard()
        reachable(By.text("Continue").enabled(true), "advanced setup: Continue", drawsWhole = "Continue", usable = true).click()

        enterCvc(TAPSIGNER_CVC, "Enter Factory CVC", "Back", "Continue", next = "Create New CVC")
        enterCvc(NEW_CVC, "Create New CVC", "Back", "Continue", next = "Confirm New CVC")

        // the card leaves the field once setup has started on it
        signer.dropNextTapSignerSession(afterExchanges = EXCHANGES_BEFORE_DROP)
        enterCvc(NEW_CVC, "Confirm New CVC", "Back", "Confirm and continue", next = "Continue Setup")
        assertScreenFits("setup retry", "Cancel", "Setup in Progress", "Continue Setup")

        // each scan can stop at another stage, which offers to continue again
        repeat(3) {
            if (device.hasObject(By.text("Setup Complete"))) return@repeat

            reachable(By.text("Continue Setup").enabled(true), "Continue Setup", usable = true).click()
            device.wait(Until.hasObject(By.text("Setup Complete")), 30_000L)
        }

        assertScreenFits("setup success", "Cancel", "Setup Complete", "Download Backup", "Continue")
        reachable(By.text("Continue"), "setup success: Continue", usable = true).click()
        device.waitFor(By.res("selectedWallet.balance"), timeoutMs = 30_000L)
    }

    // MARK: import

    private fun importCardThroughEveryScreen() {
        signer.dropNextTapSignerSession(afterExchanges = EXCHANGES_BEFORE_DROP)
        enterCvc(TAPSIGNER_CVC, "Enter TAPSIGNER CVC", "Cancel", "Submit", next = "Continue Import")
        assertScreenFits("import retry", "Cancel", "Import needs to continue", "Continue Import")
        reachable(By.text("Continue Import"), "Continue Import", usable = true).click()

        device.waitFor(By.text("Import Complete"), timeoutMs = 30_000L)
        assertScreenFits("import success", "Cancel", "Import Complete", "Continue")
        reachable(By.text("Continue"), "import success: Continue", usable = true).click()
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
            reachable(it, "$title: $submit", drawsWhole = submit, usable = true)
        }

        if (!device.wait(Until.hasObject(By.text(next)), 30_000L)) {
            scrollBackTo(By.text(next), "after $title accepted the CVC, \"$next\"")
        }
    }

    /**
     * Sets the largest system font scale and waits until the app's resumed activity has it
     *
     * The change recreates the activity, and a click that lands on the one it replaces is lost,
     * so the wallet screen must be the new activity's before the test opens Send
     */
    private fun setLargestFontScaleAndWaitForRecreation() {
        device.executeShellCommand("settings put system font_scale $LARGEST_SYSTEM_FONT_SCALE")
        wallet.waitUntil("expected the app to recreate its activity at font scale $LARGEST_SYSTEM_FONT_SCALE", timeoutMs = 30_000L) {
            var applied = false
            InstrumentationRegistry.getInstrumentation().runOnMainSync {
                applied =
                    ActivityLifecycleMonitorRegistry
                        .getInstance()
                        .getActivitiesInStage(Stage.RESUMED)
                        .any { it.resources.configuration.fontScale == LARGEST_SYSTEM_FONT_SCALE }
            }

            applied
        }

        device.waitForIdle()
    }

    /** Checks the CVC screen titled [title] fits, before any CVC is entered */
    private fun assertCvcScreenFits(
        title: String,
        back: String,
    ) {
        // the CVC field takes the focus as the screen opens, and the screen scrolls it into view,
        // which with a long message takes the title and the back action above the top, so the
        // screen counts as open once its title or its field shows, and the checks start at its top
        wallet.waitUntil("the $title screen never showed", timeoutMs = 15_000L) {
            device.hasObject(By.text(title)) || device.hasObject(By.text("CVC"))
        }

        scrollBackTo(By.text(back), "$title: \"$back\"")
        assertScreenFits(title, back, title)
        reachable(By.clazz("android.widget.EditText"), "$title: CVC field")
    }

    /**
     * Drags the screen's scroll area down, above any keyboard, until a match for [selector] shows,
     * for content above the top, which [reachable] never scrolls back to
     */
    private fun scrollBackTo(
        selector: BySelector,
        name: String,
    ) {
        repeat(MAX_NUDGES * 3) {
            if (device.hasObject(selector)) return

            val viewport =
                device
                    .findObjects(By.scrollable(true))
                    .map { it.visibleBounds }
                    .maxByOrNull { it.width() * it.height() }
                    ?: error("$name: the screen has no scroll area")

            val bottom = minOf(viewport.bottom, keyboardTop() ?: viewport.bottom)
            nudge(Rect(viewport.left, viewport.top, viewport.right, bottom), towardsTop = false)
        }

        assertTrue("$name never scrolled back into view", device.hasObject(selector))
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
        reachable(By.text(back), "$screen: \"$back\"", drawsWhole = back, usable = true)

        val titleNode = reachable(By.text(title), "$screen: \"$title\"", drawsWhole = title)
        assertWindowTextsDrawWhole(screen, titleNode.visibleBounds)
        assertWindowTextsShowWhole(screen, titleNode.visibleBounds)

        actions.forEach { reachable(By.text(it), "$screen: \"$it\"", drawsWhole = it, usable = true) }
    }

    /**
     * Fails unless every Compose text in the window that shows [titleVisible] can be scrolled to
     * show whole, with no ancestor clipping it
     *
     * A text keeps its whole layout inside a smaller container that clips it, which the layout
     * check can't see, so this compares each text's clipped bounds with its full bounds once it is
     * scrolled into its scroll area. A text taller than its scroll area must show its whole first
     * line and then its whole last line. Text fields are left out, as in [assertWindowTextsDrawWhole]
     *
     * The texts are checked from the bottom up, so the scroll areas end near their top, where the
     * later downward scroll to each action starts
     */
    private fun assertWindowTextsShowWhole(
        screen: String,
        titleVisible: Rect,
    ) {
        var root: RootForTest? = null
        var texts = emptyList<Pair<Int, String>>()

        InstrumentationRegistry.getInstrumentation().runOnMainSync {
            root = semanticsNodeAt(titleVisible)?.first
            texts =
                root
                    ?.semanticsOwner
                    ?.getAllSemanticsNodes(mergingEnabled = false)
                    ?.filter { SemanticsProperties.Text in it.config && SemanticsProperties.EditableText !in it.config }
                    ?.filter { it.size.height > 0 && it.size.width > 0 }
                    ?.sortedByDescending { it.positionOnScreen.y }
                    ?.map { node -> node.id to node.config[SemanticsProperties.Text].joinToString(" ") { it.text } }
                    .orEmpty()
        }

        val owner = checkNotNull(root) { "$screen: its title matches no Compose semantics node" }
        val problems =
            texts.mapNotNull { (id, text) ->
                textShowWholeProblem(owner, id)?.let { "\"${text.take(TEXT_IN_MESSAGE)}\": $it" }
            }

        assertTrue("$screen has texts that never show whole: $problems", problems.isEmpty())
    }

    /**
     * Where a text lies on screen, where its first and last lines lie across its width, and the
     * scroll area that can move it
     */
    private data class TextPlacement(
        val shown: Rect,
        val whole: Rect,
        val firstLine: Rect?,
        val lastLine: Rect?,
        val viewport: Rect?,
        val scrollBy: ((Float, Float) -> Boolean)?,
    )

    /**
     * Why the text with semantics [id] never shows whole, or null when it does
     *
     * Its scroll area is scrolled just far enough to bring the part under check inside it. A text
     * taller than the area must show its whole first line and then its whole last line, from the
     * line bounds its layout measures, so an area too short for a line fails
     */
    private fun textShowWholeProblem(
        root: RootForTest,
        id: Int,
    ): String? {
        val edges = mutableListOf("whole")
        var checked = 0
        var scrolledFrom: Rect? = null

        repeat(MAX_NUDGES * 2) {
            val placement = textPlacement(root, id) ?: return null
            val (shown, whole, firstLine, lastLine, viewport, scrollBy) = placement
            if (viewport != null && whole.height() > viewport.height() + CLIP_TOLERANCE_PX && edges == listOf("whole")) {
                edges.clear()
                edges += listOf("first line", "last line")
            }

            val edge = edges[checked]
            val part =
                when (edge) {
                    "first line" -> firstLine ?: return "its text reports no layout for its first line"
                    "last line" -> lastLine ?: return "its text reports no layout for its last line"
                    else -> whole
                }

            if (viewport != null && part.height() > viewport.height() + CLIP_TOLERANCE_PX) {
                return "its $edge is ${part.height()}px tall but its scroll area shows only ${viewport.height()}px"
            }

            // a part outside the area scrolls to its nearer edge, and the last line to the bottom
            val outside = viewport != null && (part.top < viewport.top || part.bottom > viewport.bottom)
            val delta =
                when {
                    viewport == null || !outside -> 0
                    edge == "last line" || part.top >= viewport.top -> part.bottom - viewport.bottom
                    else -> part.top - viewport.top
                }

            // a scroll that didn't move the text has reached the end of its area, so check it there
            if (abs(delta) > CLIP_TOLERANCE_PX && scrollBy != null && whole != scrolledFrom) {
                scrolledFrom = whole
                InstrumentationRegistry.getInstrumentation().runOnMainSync { scrollBy(0f, delta.toFloat()) }
                device.waitForIdle()
                Thread.sleep(300)
                return@repeat
            }

            val shows =
                shown.left - CLIP_TOLERANCE_PX <= part.left &&
                    shown.top - CLIP_TOLERANCE_PX <= part.top &&
                    shown.right + CLIP_TOLERANCE_PX >= part.right &&
                    shown.bottom + CLIP_TOLERANCE_PX >= part.bottom

            if (!shows) return "its ${if (edge == "whole") "text" else edge} spans $part but it shows only $shown"

            checked += 1
            scrolledFrom = null
            if (checked == edges.size) return null
        }

        return "it never scrolled to show its ${edges.getOrNull(checked)?.takeIf { it != "whole" } ?: "text"} whole"
    }

    /** The on-screen bounds of the text with semantics [id] and its nearest vertical scroll area */
    private fun textPlacement(
        root: RootForTest,
        id: Int,
    ): TextPlacement? {
        var placement: TextPlacement? = null
        val display = Rect(0, 0, device.displayWidth, device.displayHeight)

        InstrumentationRegistry.getInstrumentation().runOnMainSync {
            val node = root.semanticsOwner.getAllSemanticsNodes(mergingEnabled = false).firstOrNull { it.id == id } ?: return@runOnMainSync
            val scroller =
                generateSequence(node.parent) { it.parent }
                    .firstOrNull { SemanticsProperties.VerticalScrollAxisRange in it.config && SemanticsActions.ScrollBy in it.config }

            // boundsInWindow isn't clipped to the display, which a window can extend past
            val shown = node.clippedOnScreen().also { if (!it.intersect(display)) it.setEmpty() }
            val viewport = scroller?.clippedOnScreen()?.also { if (!it.intersect(display)) it.setEmpty() }
            val whole = node.fullOnScreen()
            val layout = node.textLayout()

            // line tops and bottoms match the drawn text, as layoutProblem explains
            val line = { index: Int ->
                layout?.let { Rect(whole.left, whole.top + it.getLineTop(index).roundToInt(), whole.right, whole.top + it.getLineBottom(index).roundToInt()) }
            }

            placement =
                TextPlacement(
                    shown = shown,
                    whole = whole,
                    firstLine = line(0),
                    lastLine = layout?.let { line(it.lineCount - 1) },
                    viewport = viewport,
                    scrollBy = scroller?.config?.getOrNull(SemanticsActions.ScrollBy)?.action,
                )
        }

        return placement
    }

    /** The text's single layout, as the accessibility service reads it, or null when it has none */
    private fun SemanticsNode.textLayout(): TextLayoutResult? {
        val action = config.getOrNull(SemanticsActions.GetTextLayoutResult)?.action ?: return null
        val layouts = mutableListOf<TextLayoutResult>()

        return layouts.takeIf { action(it) }?.singleOrNull()?.takeIf { it.lineCount > 0 }
    }

    /**
     * Scrolls until the node's control lies above the gesture strip and shows whole, then checks the
     * node isn't squeezed
     *
     * The control is the clickable that merges the node, such as a card holding a title and a
     * description, or the node itself when nothing does. UiAutomator reports bounds already clipped
     * to the display, window and scroll container, so the control's full and shown bounds come from
     * the Compose semantics tree instead. A control cut off at the top or bottom is dragged inward
     * until it shows whole; one cut off at the side, or that never shows whole, fails
     *
     * With [drawsWhole], the control's text showing that string must also lay out whole, and with
     * [usable], the control must take a click and be enabled
     */
    private fun reachable(
        selector: BySelector,
        name: String,
        drawsWhole: String? = null,
        usable: Boolean = false,
    ): UiObject2 {
        val density = InstrumentationRegistry.getInstrumentation().targetContext.resources.displayMetrics.density
        val gestureStripTop = device.displayHeight - (GESTURE_STRIP_DP * density).toInt()
        var node = device.scrollTo(selector, timeoutMs = 15_000L)
        var control: ControlBounds? = null

        repeat(MAX_NUDGES) {
            // a recomposition can replace the node between reads
            val visible =
                try {
                    node.visibleBounds
                } catch (_: StaleObjectException) {
                    node = device.waitFor(selector, timeoutMs = 5_000L)
                    node.visibleBounds
                }

            val (shown, whole) =
                controlBounds(visible)?.also { control = it } ?: error("$name at $visible matches no Compose semantics node")

            val cutTop = whole.top < shown.top - CLIP_TOLERANCE_PX
            val cutBottom = whole.bottom > shown.bottom + CLIP_TOLERANCE_PX
            assertTrue(
                "$name is cut off at the side: its control spans $whole but shows only $shown",
                whole.left >= shown.left - CLIP_TOLERANCE_PX && whole.right <= shown.right + CLIP_TOLERANCE_PX,
            )
            assertTrue("$name's control spans $whole, taller than the $shown it can show in", !(cutTop && cutBottom))

            if (!cutTop && !cutBottom && shown.bottom <= gestureStripTop) {
                assertTrue(
                    "$name is squeezed to $visible, under ${MIN_NODE_HEIGHT_DP}dp tall",
                    visible.height() >= MIN_NODE_HEIGHT_DP * density,
                )

                drawsWhole?.let { assertDrawsWhole(visible, it, name) }
                if (usable) assertUsable(visible, name)
                return node
            }

            // below the gesture strip counts as cut off at the bottom, since the content must move up
            nudge(node.scrollViewport() ?: Rect(0, 0, device.displayWidth, device.displayHeight), towardsTop = !cutTop)
            node = device.waitFor(selector, timeoutMs = 5_000L)
        }

        error("$name never showed whole after $MAX_NUDGES drags: ${control ?: "no control found"}")
    }

    /**
     * Waits for the keyboard to come to rest, then drags the scroll area above it until the control
     * matching [selector] shows whole there, and fails if it never does
     *
     * With [toolbar], the space ends at the top of that bar on the keyboard, or at the keyboard's
     * top if that is higher, so a control behind the bar fails too. Every measurement is taken once
     * the window's keyboard animations have ended, see [settledPlacement], so a keyboard still
     * sliding open can't pass or fail the check. The drags stay in that space, where a drag on the
     * keys would type instead of scroll
     */
    private fun scrollAboveKeyboard(
        selector: BySelector,
        name: String,
        toolbar: BySelector? = null,
    ) {
        val cover = if (toolbar == null) "keyboard" else "keyboard and its toolbar"
        var last: String? = null

        repeat(MAX_NUDGES) {
            val placement = settledPlacement(selector, toolbar, "expected the $cover to come to rest before $name")
            val bounds = placement.control
            if (bounds != null && bounds.whole.bottom <= placement.coverTop && bounds.shown.contains(bounds.whole)) return

            last = "$bounds with the $cover from ${placement.coverTop}"
            val viewport =
                device
                    .findObjects(By.scrollable(true))
                    .map { it.visibleBounds }
                    .maxByOrNull { it.width() * it.height() }
                    ?: error("$name: the screen has no scroll area")

            nudge(Rect(viewport.left, viewport.top, viewport.right, minOf(viewport.bottom, placement.coverTop)), towardsTop = true)
        }

        error("$name never showed whole above the $cover after $MAX_NUDGES drags: ${last ?: "not found"}")
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
     * top, within [KEYBOARD_EDGE_ROUNDING_PX]. The agreeing second read lets the frame that applies
     * the final insets land first
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
        val keyboard = keyboardTop() ?: return null

        val bar =
            toolbar?.let {
                device
                    .findObject(it)
                    ?.visibleBounds
                    ?.takeIf { bounds -> !bounds.isEmpty && abs(bounds.bottom - keyboard) <= KEYBOARD_EDGE_ROUNDING_PX }
                    ?: return null
            }

        return KeyboardPlacement(keyboard, bar, device.findObject(selector)?.visibleBounds?.let(::controlBounds), ended)
    }

    /** The top of the keyboard on screen, or null while it is closed */
    private fun keyboardTop(): Int? {
        var top: Int? = null

        InstrumentationRegistry.getInstrumentation().runOnMainSync {
            val decor =
                ActivityLifecycleMonitorRegistry
                    .getInstance()
                    .getActivitiesInStage(Stage.RESUMED)
                    .firstOrNull()
                    ?.window
                    ?.decorView

            val insets = decor?.rootWindowInsets?.takeIf { it.isVisible(WindowInsets.Type.ime()) } ?: return@runOnMainSync
            val location = IntArray(2).also(decor::getLocationOnScreen)
            top = location[1] + decor.height - insets.getInsets(WindowInsets.Type.ime()).bottom
        }

        return top
    }

    /** Where a control shows on screen after every clip, and where it would lie unclipped */
    private data class ControlBounds(
        val shown: Rect,
        val whole: Rect,
    )

    /** The bounds of the control holding the Compose node that UiAutomator shows at [visible] */
    private fun controlBounds(visible: Rect): ControlBounds? {
        var bounds: ControlBounds? = null

        InstrumentationRegistry.getInstrumentation().runOnMainSync {
            val control = semanticsNodeAt(visible)?.second?.controlOrSelf() ?: return@runOnMainSync
            val shown = control.clippedOnScreen()

            // boundsInWindow isn't clipped to the display, which a window can extend past
            if (!shown.intersect(0, 0, device.displayWidth, device.displayHeight)) shown.setEmpty()
            bounds = ControlBounds(shown, control.fullOnScreen())
        }

        return bounds
    }

    /**
     * Fails unless every Compose text in the control UiAutomator shows at [visible] that reads
     * [text] lays out whole: no line ellipsized, no line past the text's height or line limit, and
     * no line wider than the text
     *
     * UiAutomator reports a text's whole string even when it draws cut short, so the layout comes
     * from Compose. A text that reports no layout fails too, so a change that hides the layout
     * can't turn the check off
     */
    private fun assertDrawsWhole(
        visible: Rect,
        text: String,
        name: String,
    ) {
        var problem: String? = null

        InstrumentationRegistry.getInstrumentation().runOnMainSync {
            val control = semanticsNodeAt(visible)?.second?.controlOrSelf()
            val texts =
                control
                    ?.selfAndDescendants()
                    ?.filter { node -> node.config.getOrNull(SemanticsProperties.Text).orEmpty().any { it.text == text } }
                    ?.toList()
                    .orEmpty()

            problem =
                when {
                    control == null -> "matches no Compose semantics node"
                    texts.isEmpty() -> "has no Compose text reading \"$text\" in its control"
                    else -> texts.firstNotNullOfOrNull { it.layoutProblem() }
                }
        }

        problem?.let { error("$name doesn't draw whole: $it") }
    }

    /**
     * Fails unless every Compose text in the window that shows [titleVisible] lays out whole
     *
     * The TAPSIGNER screens are a sheet in their own window, so this covers each screen's
     * descriptions without naming them. Text fields are left out, since they scroll a long entry
     * inside themselves by design
     */
    private fun assertWindowTextsDrawWhole(
        screen: String,
        titleVisible: Rect,
    ) {
        var problems = listOf("its title matches no Compose semantics node")

        InstrumentationRegistry.getInstrumentation().runOnMainSync {
            val root = semanticsNodeAt(titleVisible)?.first ?: return@runOnMainSync
            problems =
                root.semanticsOwner
                    .getAllSemanticsNodes(mergingEnabled = false)
                    .filter { SemanticsProperties.Text in it.config && SemanticsProperties.EditableText !in it.config }
                    .mapNotNull { node ->
                        val text = node.config[SemanticsProperties.Text].joinToString(" ") { it.text }
                        node.layoutProblem()?.let { "\"${text.take(TEXT_IN_MESSAGE)}\": $it" }
                    }
        }

        assertTrue("$screen has texts that don't draw whole: $problems", problems.isEmpty())
    }

    /** Fails unless the control UiAutomator shows at [visible] takes a click and is enabled */
    private fun assertUsable(
        visible: Rect,
        name: String,
    ) {
        var problem: String? = null

        InstrumentationRegistry.getInstrumentation().runOnMainSync {
            val control = semanticsNodeAt(visible)?.second?.controlOrSelf()
            problem =
                when {
                    control == null -> "matches no Compose semantics node"
                    SemanticsActions.OnClick !in control.config -> "takes no click"
                    SemanticsProperties.Disabled in control.config -> "is disabled"
                    else -> null
                }
        }

        problem?.let { error("$name $it") }
    }

    /**
     * Why the text's layout doesn't show whole, or null when it does
     *
     * A text node reports its layout through this action, as the accessibility service reads it.
     * Compose lays that copy out again at the text's maximum width constraint rather than the width
     * it took, so its paragraph width is the parent's and `didOverflowWidth` holds for any text
     * narrower than its parent. Line breaks and heights match the drawn text, since a text only
     * takes less than its maximum width when it fits on its lines without wrapping, so the width
     * check measures each line instead
     */
    private fun SemanticsNode.layoutProblem(): String? {
        val action = config.getOrNull(SemanticsActions.GetTextLayoutResult)?.action ?: return "its text reports no layout"
        val layouts = mutableListOf<TextLayoutResult>()
        if (!action(layouts)) return "its text has no layout yet"

        val layout = layouts.singleOrNull() ?: return "its text reports ${layouts.size} layouts"
        val lines = 0 until layout.lineCount
        val ellipsized = lines.filter(layout::isLineEllipsized)
        val widest = lines.maxOfOrNull { layout.getLineRight(it) - layout.getLineLeft(it) } ?: 0f

        return when {
            ellipsized.isNotEmpty() -> "line ${ellipsized.first() + 1} of ${layout.lineCount} is ellipsized"
            layout.multiParagraph.didExceedMaxLines -> "it needs more than its ${layout.lineCount} line limit"
            layout.didOverflowHeight -> "it needs ${layout.multiParagraph.height}px but gets ${layout.size.height}px"
            widest > layout.size.width + LINE_WIDTH_TOLERANCE_PX -> "a line needs ${widest}px wide but gets ${layout.size.width}px"
            else -> null
        }
    }

    /**
     * The Compose node UiAutomator shows at [visible], read on the main thread
     *
     * Every node's clipped bounds are compared with [visible], and the closest one is taken, so the
     * same text elsewhere, such as behind the sheet, doesn't match
     */
    private fun semanticsNodeAt(visible: Rect): Pair<RootForTest, SemanticsNode>? =
        WindowInspector
            .getGlobalWindowViews()
            .flatMap { it.composeRoots() }
            .flatMap { root -> root.semanticsOwner.getAllSemanticsNodes(mergingEnabled = false).map { root to it } }
            .filter { (_, node) -> Rect.intersects(node.clippedOnScreen(), visible) }
            .minByOrNull { (_, node) -> edgeDistance(node.clippedOnScreen(), visible) }

    private fun SemanticsNode.selfAndDescendants(): Sequence<SemanticsNode> =
        sequenceOf(this) + children.asSequence().flatMap { it.selfAndDescendants() }

    // a clickable that merges its children is one control, such as a card with a title and a description
    private fun SemanticsNode.controlOrSelf(): SemanticsNode =
        generateSequence(this) { it.parent }
            .firstOrNull { it.config.isMergingSemanticsOfDescendants && SemanticsActions.OnClick in it.config }
            ?: this

    private fun View.composeRoots(): List<RootForTest> =
        when (this) {
            is RootForTest -> listOf(this)
            is ViewGroup -> (0 until childCount).flatMap { getChildAt(it).composeRoots() }
            else -> emptyList()
        }

    // boundsInWindow is clipped by every ancestor; shifting it by the window's screen offset puts
    // it beside the unclipped position on screen
    private fun SemanticsNode.clippedOnScreen(): Rect {
        val offset = positionOnScreen - positionInWindow
        return boundsInWindow.translate(offset).rounded()
    }

    private fun SemanticsNode.fullOnScreen(): Rect = ComposeRect(positionOnScreen, size.toSize()).rounded()

    // rounds each edge to the nearest pixel, as the accessibility bounds do
    private fun ComposeRect.rounded(): Rect = Rect(left.roundToInt(), top.roundToInt(), right.roundToInt(), bottom.roundToInt())

    private fun edgeDistance(
        a: Rect,
        b: Rect,
    ): Int = abs(a.left - b.left) + abs(a.top - b.top) + abs(a.right - b.right) + abs(a.bottom - b.bottom)

    // the nearest scrollable ancestor's visible bounds, which is where a drag scrolls the node
    private fun UiObject2.scrollViewport(): Rect? =
        generateSequence(parent) { it.parent }
            .firstOrNull { it.isScrollable }
            ?.visibleBounds

    // a short slow drag, which moves the content without a fling
    private fun nudge(
        viewport: Rect,
        towardsTop: Boolean,
    ) {
        val density = InstrumentationRegistry.getInstrumentation().targetContext.resources.displayMetrics.density
        val distance = minOf(viewport.height() / 4, (NUDGE_DP * density).toInt())
        val (from, to) =
            if (towardsTop) {
                viewport.centerY() + distance / 2 to viewport.centerY() - distance / 2
            } else {
                viewport.centerY() - distance / 2 to viewport.centerY() + distance / 2
            }

        device.swipe(viewport.centerX(), from, viewport.centerX(), to, 40)
        device.waitForIdle()
        Thread.sleep(300)
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
