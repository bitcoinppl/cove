package org.bitcoinppl.cove.test

import android.graphics.Rect
import android.view.View
import android.view.ViewGroup
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.wrapContentHeight
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.key
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.runtime.snapshotFlow
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clipToBounds
import androidx.compose.ui.node.RootForTest
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.semantics.SemanticsNode
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.semantics.getAllSemanticsNodes
import androidx.compose.ui.semantics.getOrNull
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import androidx.test.core.app.ActivityScenario
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import androidx.test.uiautomator.UiDevice
import kotlinx.coroutines.flow.first
import org.junit.After
import org.junit.Assert.assertThrows
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import kotlin.math.roundToInt

/**
 * Drives [FullLaunchLayoutChecks.assertWindowTextsShowWhole] on texts that its own scroll
 * replaces, since a text found again by its string alone could be measured in place of another
 * that reads the same
 *
 * Each screen has a short scroll area whose texts start outside it, so the check scrolls them in,
 * and the scroll replaces or removes them. A clipped text keeps its whole layout inside a box
 * shorter than it, so only the show-whole check can catch it. The screens run in a plain activity
 * rather than a Compose test rule, whose test frame clock would hold the check's scroll animation
 * still, so frames come from the display as on the full-launch screens
 */
@RunWith(AndroidJUnit4::class)
class FullLaunchLayoutChecksTest {
    companion object {
        private const val TITLE = "Layout checks"
        private const val SAME = "The same text"
        private const val LONE = "A text of its own"
    }

    private lateinit var device: UiDevice
    private lateinit var checks: FullLaunchLayoutChecks
    private var scenario: ActivityScenario<ComponentActivity>? = null

    @Before
    fun setUp() {
        device = UiDevice.getInstance(InstrumentationRegistry.getInstrumentation())
        checks = FullLaunchLayoutChecks(device, FullLaunchWalletRobot(device))
    }

    @After
    fun closeScreen() {
        scenario?.close()
    }

    @Test
    fun aReplacedTextIsFoundAgainAndChecked() {
        showScrolling { moved -> key(moved) { Text(SAME) } }
        val before = textIds(SAME)

        checks.assertWindowTextsShowWhole("replaced text", titleBounds())
        assertScrollReplaced(SAME, before)
    }

    @Test
    fun aClippedTextThatLeavesWithNoTwinFails() {
        showScrolling { moved -> if (!moved) ClippedText(LONE) }
        val before = textIds(LONE)

        val failure = assertThrows(AssertionError::class.java) { checks.assertWindowTextsShowWhole("lone text", titleBounds()) }
        assertTrue("expected the text to have left its window: ${failure.message}", "left its window" in failure.message.orEmpty())
        assertScrollReplaced(LONE, before)
    }

    @Test
    fun aClippedTextThatLeavesFailsThoughAnotherReadsTheSame() {
        showScrolling { moved ->
            Text(SAME)
            if (!moved) ClippedText(SAME)
        }

        val before = textIds(SAME)

        assertAmbiguous("left text")
        assertScrollReplaced(SAME, before)
    }

    @Test
    fun aClippedTextReplacedBesideAnotherThatReadsTheSameFails() {
        showScrolling { moved ->
            key(moved) {
                Text(SAME)
                ClippedText(SAME)
            }
        }

        val before = textIds(SAME)

        assertAmbiguous("replaced pair")
        assertScrollReplaced(SAME, before)
    }

    // the shown text is checked first, from the bottom up, and the scroll up to the clipped one
    // above the area removes it and replaces the shown one, its only new twin
    @Test
    fun aClippedTextAboveFailsWhenTheScrollReplacesACheckedTwin() {
        showScrolling(start = 400.dp, lead = 360.dp) { moved ->
            if (!moved) ClippedText(SAME)
            Spacer(Modifier.height(36.dp))
            key(moved) { Text(SAME) }
        }

        val before = textIds(SAME)

        assertAmbiguous("scan order")
        assertScrollReplaced(SAME, before)
    }

    private fun assertAmbiguous(screen: String) {
        val failure = assertThrows(AssertionError::class.java) { checks.assertWindowTextsShowWhole(screen, titleBounds()) }
        assertTrue("expected an ambiguous replacement: ${failure.message}", "ambiguous" in failure.message.orEmpty())
    }

    // the check's scroll must have removed or replaced a text, or the screen proved nothing
    private fun assertScrollReplaced(
        text: String,
        before: Set<Int>,
    ) {
        val after = textIds(text)
        assertTrue("the scroll replaced no \"$text\": $before then $after", before.isNotEmpty() && (before - after).isNotEmpty())
    }

    /**
     * Shows a title above a 200dp scroll area that starts scrolled to [start], with [lead] of
     * space above [content], which learns once the area has moved and stays told
     */
    private fun showScrolling(
        start: Dp = 0.dp,
        lead: Dp = 400.dp,
        content: @Composable ColumnScope.(moved: Boolean) -> Unit,
    ) {
        scenario =
            ActivityScenario.launch(ComponentActivity::class.java).onActivity { activity ->
                activity.setContent {
                    val startPx = with(LocalDensity.current) { start.roundToPx() }
                    val scroll = rememberScrollState(startPx)
                    var moved by remember { mutableStateOf(false) }

                    LaunchedEffect(scroll) {
                        snapshotFlow { scroll.value }.first { it != startPx }
                        moved = true
                    }

                    Column(Modifier.fillMaxSize()) {
                        Text(TITLE)
                        Column(Modifier.height(200.dp).verticalScroll(scroll)) {
                            Spacer(Modifier.height(lead))
                            content(moved)
                            Spacer(Modifier.height(400.dp))
                        }
                    }
                }
            }

        titleBounds()
        device.waitForIdle()
    }

    /**
     * Where the title shows on screen, read from the activity's semantics, as the check reads
     * every window, since UiAutomator didn't reliably see this activity's window
     */
    private fun titleBounds(): Rect {
        repeat(100) {
            var bounds: Rect? = null

            checkNotNull(scenario).onActivity { activity ->
                bounds =
                    activity
                        .textNodes(TITLE)
                        .firstOrNull()
                        ?.let { node ->
                            val shown = node.boundsInWindow.translate(node.positionOnScreen - node.positionInWindow)
                            Rect(shown.left.roundToInt(), shown.top.roundToInt(), shown.right.roundToInt(), shown.bottom.roundToInt())
                        }?.takeUnless { it.isEmpty }
            }

            bounds?.let { return it }
            Thread.sleep(100)
        }

        error("the title never showed")
    }

    private fun textIds(text: String): Set<Int> {
        var ids = emptySet<Int>()
        checkNotNull(scenario).onActivity { activity -> ids = activity.textNodes(text).map { it.id }.toSet() }

        return ids
    }

    private fun ComponentActivity.textNodes(text: String): List<SemanticsNode> =
        window.decorView
            .composeRoots()
            .flatMap { it.semanticsOwner.getAllSemanticsNodes(mergingEnabled = false) }
            .filter { node -> node.config.getOrNull(SemanticsProperties.Text)?.joinToString(" ") { it.text } == text }

    private fun View.composeRoots(): List<RootForTest> =
        when (this) {
            is RootForTest -> listOf(this)
            is ViewGroup -> (0 until childCount).flatMap { getChildAt(it).composeRoots() }
            else -> emptyList()
        }

    // the text lays out whole at its own height, but the box around it shows only 4dp of it
    @Composable
    private fun ClippedText(text: String) {
        Box(Modifier.height(4.dp).clipToBounds()) {
            Text(text, Modifier.wrapContentHeight(unbounded = true))
        }
    }
}
