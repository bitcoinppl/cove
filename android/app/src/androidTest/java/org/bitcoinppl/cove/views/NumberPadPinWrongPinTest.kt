package org.bitcoinppl.cove.views

import androidx.compose.ui.geometry.Rect
import androidx.compose.ui.semantics.SemanticsNode
import androidx.compose.ui.test.ComposeTimeoutException
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performScrollTo
import androidx.compose.ui.unit.toSize
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
private const val PIN_LENGTH = 6

/**
 * Every wrong PIN shakes and clears the PIN boxes where the user can see them
 *
 * On a landscape phone at the largest font size the pad is taller than the screen, so reaching
 * its bottom row scrolls the PIN boxes out of view. A wrong PIN typed there must bring them back
 * into view, or the pad looks like it ignored the PIN
 */
@LayoutRegressionTest
class NumberPadPinWrongPinTest {
    @get:Rule
    val compose = coveComposeRule()

    private val wrongPins: MutableList<String> = Collections.synchronizedList(mutableListOf())
    private val failures = mutableListOf<String>()

    @Test
    fun everyWrongPinShowsThePinBoxes() {
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

        repeat(2) { attempt ->
            val zero = compose.onNodeWithTag("numberPadPin.digit.0")
            zero.performScrollTo()

            // the boxes must start out of view, or this test can't tell whether the pad shows them
            assertTrue(
                "attempt ${attempt + 1}: reaching 0 should scroll the PIN boxes out of view, so the test checks the pad brings them back",
                !pinBoxesInView(),
            )

            repeat(PIN_LENGTH) { zero.performClick() }

            try {
                compose.waitUntil(timeoutMillis = 5_000) { wrongPins.size == attempt + 1 }
            } catch (_: ComposeTimeoutException) {
                failures += "attempt ${attempt + 1}: the wrong PIN should shake the PIN boxes, but the pad didn't react"
            }
            compose.waitForIdle()

            if (!pinBoxesInView()) failures += "attempt ${attempt + 1}: the wrong PIN should show the PIN boxes, but they stayed out of view"
        }

        assertTrue(failures.joinToString("\n"), failures.isEmpty())
    }

    private fun pinBoxesInView(): Boolean {
        val viewport = compose.onNodeWithTag(VIEWPORT_TAG).fetchSemanticsNode().boundsInRoot
        val pinBoxes = compose.onNodeWithTag("numberPadPin.pinBoxes").fetchSemanticsNode().ownBoundsPx()

        return pinBoxes.top >= viewport.top && pinBoxes.bottom <= viewport.bottom
    }
}

private fun SemanticsNode.ownBoundsPx(): Rect = Rect(positionInRoot, size.toSize())
