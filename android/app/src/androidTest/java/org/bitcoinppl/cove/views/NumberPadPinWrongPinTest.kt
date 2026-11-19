package org.bitcoinppl.cove.views

import android.view.KeyEvent
import androidx.compose.ui.geometry.Rect
import androidx.compose.ui.semantics.SemanticsNode
import androidx.compose.ui.test.hasAnyAncestor
import androidx.compose.ui.test.hasTestTag
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performScrollTo
import androidx.compose.ui.test.performTouchInput
import androidx.compose.ui.test.requestFocus
import androidx.compose.ui.test.swipeUp
import androidx.compose.ui.unit.toSize
import androidx.test.platform.app.InstrumentationRegistry
import org.bitcoinppl.cove.test.CompactViewport
import org.bitcoinppl.cove.test.LARGEST_SYSTEM_FONT_SCALE
import org.bitcoinppl.cove.test.LayoutRegressionTest
import org.bitcoinppl.cove.test.Viewport
import org.bitcoinppl.cove.test.coveComposeRule
import org.bitcoinppl.cove.ui.theme.CoveTheme
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import java.util.Collections

private const val VIEWPORT_TAG = "numberPadPinWrongPin.viewport"
private const val PIN_BOXES_TAG = "numberPadPin.pinBoxes"
private const val PIN_LENGTH = 6

// the feedback takes about a second, so five seconds of frames means the pad never cleared the PIN
private const val MAX_FEEDBACK_FRAMES = 300

// the shake takes about half a second, so the boxes must stay still this long after they clear
private const val FRAMES_AFTER_CLEAR = 60

/**
 * Every wrong PIN shakes and then clears the PIN boxes where the user can see them
 *
 * On a landscape phone at the largest font size the pad is taller than the screen, so reaching
 * its bottom row scrolls the PIN boxes out of view. A wrong PIN typed there must bring them back
 * into view before they shake, and clear them only after, or the pad looks like it ignored the
 * PIN. The test steps the clock one frame at a time, so it sees the order whatever the speed, and
 * a swipe or a hardware keyboard's Tab while the boxes scroll back or shake must not move them out
 * of view
 */
@LayoutRegressionTest
class NumberPadPinWrongPinTest {
    @get:Rule
    val compose = coveComposeRule()

    private val wrongPins: MutableList<String> = Collections.synchronizedList(mutableListOf())
    private val failures = mutableListOf<String>()

    @Test
    fun everyWrongPinShakesAndClearsThePinBoxesInView() {
        setContent()

        checkWrongPin(attempt = 1)
        checkWrongPin(attempt = 2)

        assertTrue(failures.joinToString("\n"), failures.isEmpty())
    }

    @Test
    fun aSwipeDuringTheFeedbackKeepsThePinBoxesInView() {
        setContent()

        checkWrongPin(attempt = 1, interruption = Interruption.SWIPE_WHILE_SCROLLING_BACK)
        checkWrongPin(attempt = 2, interruption = Interruption.SWIPE_WHILE_SHAKING)
        checkWrongPin(attempt = 3)

        assertTrue(failures.joinToString("\n"), failures.isEmpty())
    }

    @Test
    fun aKeyboardTabDuringTheFeedbackKeepsThePinBoxesInView() {
        setContent()

        checkWrongPin(attempt = 1, interruption = Interruption.TAB_WHILE_SCROLLING_BACK)
        checkWrongPin(attempt = 2, interruption = Interruption.TAB_WHILE_SHAKING)

        assertTrue(failures.joinToString("\n"), failures.isEmpty())
    }

    private fun setContent() {
        compose.setContent {
            CoveTheme(darkTheme = true) {
                CompactViewport(Viewport.LANDSCAPE, LARGEST_SYSTEM_FONT_SCALE, VIEWPORT_TAG) {
                    NumberPadPinView(
                        title = "Enter Current PIN",
                        isPinCorrect = { false },
                        pinLength = PIN_LENGTH,
                        backAction = {},
                        onWrongPin = { wrongPins += it },
                    )
                }
            }
        }
    }

    /**
     * Types a wrong PIN on the bottom row, by touch or, for a Tab [interruption], with a hardware
     * keyboard's Enter, and runs the [interruption] once during the feedback
     */
    private fun checkWrongPin(
        attempt: Int,
        interruption: Interruption? = null,
    ) {
        val zero = compose.onNodeWithTag("numberPadPin.digit.0")
        val usesKeyboard = interruption?.usesKeyboard == true

        // a key press leaves touch mode, so 0 can take focus as it does for a keyboard user
        if (usesKeyboard) {
            pressKey(KeyEvent.KEYCODE_TAB)
            zero.requestFocus()
        }

        zero.performScrollTo()
        compose.waitForIdle()

        // the boxes must start out of view, or this test can't tell whether the pad shows them
        assertTrue(
            "attempt $attempt: reaching 0 should scroll the PIN boxes out of view, so the test checks the pad brings them back",
            !pinBoxesInView(),
        )

        // the shake moves the boxes sideways from here, and the scroll back moves them down
        val rest = pinBoxes()
        val feedback = WrongPinFeedback()
        var interrupted = interruption == null

        compose.mainClock.autoAdvance = false
        repeat(PIN_LENGTH) { if (usesKeyboard) pressKey(KeyEvent.KEYCODE_ENTER) else zero.performClick() }

        while (feedback.framesAfterClear < FRAMES_AFTER_CLEAR && feedback.frames < MAX_FEEDBACK_FRAMES) {
            compose.mainClock.advanceTimeByFrame()

            // dragging the keypad back up, or tabbing from 0 to Delete, would scroll the boxes away
            val now = pinBoxes()
            val moving = if (interruption?.whileShaking == true) now.left != rest.left else now.top != rest.top

            if (!interrupted && moving) {
                interrupt(interruption)
                interrupted = true
            }

            feedback.observe(shaking = pinBoxes().left != rest.left, inView = pinBoxesInView(), filled = filledPinBoxes())
        }

        compose.mainClock.autoAdvance = true
        compose.waitForIdle()

        if (!interrupted) failures += "attempt $attempt: the boxes never moved, so ${interruption?.name?.lowercase()} never ran"
        feedback.failures().forEach { failures += "attempt $attempt: $it" }
        if (wrongPins.size != attempt) failures += "attempt $attempt: the pad should report each wrong PIN, but it reported ${wrongPins.size}"
    }

    private fun interrupt(interruption: Interruption?) {
        if (interruption?.usesKeyboard == true) {
            pressKey(KeyEvent.KEYCODE_TAB)
        } else {
            compose.onNodeWithTag("numberPadPin.viewport").performTouchInput { swipeUp() }
        }
    }

    private fun pressKey(keyCode: Int) = InstrumentationRegistry.getInstrumentation().sendKeyDownUpSync(keyCode)

    private fun pinBoxes(): Rect = compose.onNodeWithTag(PIN_BOXES_TAG).fetchSemanticsNode().ownBoundsPx()

    private fun pinBoxesInView(): Boolean {
        val viewport = compose.onNodeWithTag(VIEWPORT_TAG).fetchSemanticsNode().boundsInRoot
        val pinBoxes = pinBoxes()

        return pinBoxes.top >= viewport.top && pinBoxes.bottom <= viewport.bottom
    }

    private fun filledPinBoxes(): Int =
        compose
            .onAllNodes(hasText("●") and hasAnyAncestor(hasTestTag(PIN_BOXES_TAG)), useUnmergedTree = true)
            .fetchSemanticsNodes()
            .size
}

/** What the user does once during a wrong PIN's feedback, which mustn't move the boxes out of view */
private enum class Interruption(
    val usesKeyboard: Boolean,
    val whileShaking: Boolean,
) {
    SWIPE_WHILE_SCROLLING_BACK(usesKeyboard = false, whileShaking = false),
    SWIPE_WHILE_SHAKING(usesKeyboard = false, whileShaking = true),
    TAB_WHILE_SCROLLING_BACK(usesKeyboard = true, whileShaking = false),
    TAB_WHILE_SHAKING(usesKeyboard = true, whileShaking = true),
}

/** What one wrong PIN's feedback showed, frame by frame, until the boxes settled after clearing */
private class WrongPinFeedback {
    var frames = 0
        private set
    var framesAfterClear = 0
        private set

    private var showedPin = false
    private var cleared = false
    private var shookInView = false
    private var shookOutOfView = false
    private var shookBeforeClear = false
    private var shookAfterClear = false
    private var clearedInView = false

    fun observe(
        shaking: Boolean,
        inView: Boolean,
        filled: Int,
    ) {
        frames += 1
        if (cleared) framesAfterClear += 1
        if (filled == PIN_LENGTH) showedPin = true

        if (showedPin && filled == 0 && !cleared) {
            cleared = true
            clearedInView = inView
            shookBeforeClear = shookInView
        }

        if (shaking && inView) shookInView = true
        if (shaking && !inView) shookOutOfView = true

        // the boxes can pass through their resting place mid-shake, so boxes off their resting
        // place on the frame they clear, or any frame after, cleared before the shake ended
        if (shaking && cleared) shookAfterClear = true
    }

    fun failures(): List<String> =
        buildList {
            if (!cleared) add("the wrong PIN should clear the boxes, but they still held it after $frames frames")
            if (shookOutOfView) add("the boxes should shake only once they are in view, but they shook out of view")
            if (!shookInView) add("the boxes should shake where the user can see them, but they never did")
            if (cleared && !clearedInView) add("the boxes should clear in view, but they cleared out of view")
            if (cleared && (!shookBeforeClear || shookAfterClear)) {
                add("the boxes should finish shaking before they clear, but they cleared first")
            }
        }
}

private fun SemanticsNode.ownBoundsPx(): Rect = Rect(positionInRoot, size.toSize())
