package org.bitcoinppl.cove.test

import android.graphics.Rect
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.wrapContentHeight
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.key
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clipToBounds
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.unit.dp
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import androidx.test.uiautomator.By
import androidx.test.uiautomator.UiDevice
import org.junit.Assert.assertThrows
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

/**
 * Drives [FullLaunchLayoutChecks.assertWindowTextsShowWhole] on texts that its own scroll
 * replaces, since a text found again by its string alone could be measured in place of another
 * that reads the same
 *
 * Each screen has a short scroll area whose texts start below it, so the check scrolls them in,
 * and the scroll replaces them. A clipped text keeps its whole layout inside a box shorter than
 * it, so only the show-whole check can catch it
 */
@RunWith(AndroidJUnit4::class)
class FullLaunchLayoutChecksTest {
    companion object {
        private const val TITLE = "Layout checks"
        private const val SAME = "The same text"
    }

    @get:Rule
    val composeRule = createComposeRule()

    private lateinit var device: UiDevice
    private lateinit var checks: FullLaunchLayoutChecks

    @Before
    fun setUp() {
        device = UiDevice.getInstance(InstrumentationRegistry.getInstrumentation())
        checks = FullLaunchLayoutChecks(device, FullLaunchWalletRobot(device))
    }

    @Test
    fun aReplacedTextIsFoundAgainAndChecked() {
        showScrolling { scrolled -> key(scrolled) { Text(SAME) } }

        checks.assertWindowTextsShowWhole("replaced text", titleBounds())
    }

    @Test
    fun aClippedTextThatLeavesFailsThoughAnotherReadsTheSame() {
        showScrolling { scrolled ->
            Text(SAME)
            if (!scrolled) ClippedText(SAME)
        }

        val failure = assertThrows(AssertionError::class.java) { checks.assertWindowTextsShowWhole("left text", titleBounds()) }
        assertTrue("expected the clipped text to have left: ${failure.message}", "left its window" in failure.message.orEmpty())
    }

    @Test
    fun aClippedTextReplacedBesideAnotherThatReadsTheSameFails() {
        showScrolling { scrolled ->
            key(scrolled) {
                Text(SAME)
                ClippedText(SAME)
            }
        }

        val failure = assertThrows(AssertionError::class.java) { checks.assertWindowTextsShowWhole("ambiguous text", titleBounds()) }
        assertTrue("expected an ambiguous replacement: ${failure.message}", "ambiguous" in failure.message.orEmpty())
    }

    // the texts start below a 200dp scroll area, and [content] learns once it has scrolled
    private fun showScrolling(content: @Composable (scrolled: Boolean) -> Unit) {
        composeRule.setContent {
            val scroll = rememberScrollState()

            Column(Modifier.fillMaxSize()) {
                Text(TITLE)
                Column(Modifier.height(200.dp).verticalScroll(scroll)) {
                    Spacer(Modifier.height(400.dp))
                    content(scroll.value > 0)
                    Spacer(Modifier.height(400.dp))
                }
            }
        }

        composeRule.waitForIdle()
    }

    private fun titleBounds(): Rect = checkNotNull(device.findObject(By.text(TITLE))) { "the title never showed" }.visibleBounds

    // the text lays out whole at its own height, but the box around it shows only 4dp of it
    @Composable
    private fun ClippedText(text: String) {
        Box(Modifier.height(4.dp).clipToBounds()) {
            Text(text, Modifier.wrapContentHeight(unbounded = true))
        }
    }
}
