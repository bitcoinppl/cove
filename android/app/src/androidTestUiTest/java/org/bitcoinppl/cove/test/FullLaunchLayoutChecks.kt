package org.bitcoinppl.cove.test

import android.graphics.Rect
import android.view.View
import android.view.ViewGroup
import android.view.WindowInsets
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
import androidx.test.platform.app.InstrumentationRegistry
import androidx.test.runner.lifecycle.ActivityLifecycleMonitorRegistry
import androidx.test.runner.lifecycle.Stage
import androidx.test.uiautomator.By
import androidx.test.uiautomator.BySelector
import androidx.test.uiautomator.StaleObjectException
import androidx.test.uiautomator.UiDevice
import androidx.test.uiautomator.UiObject2
import org.junit.Assert.assertTrue
import kotlin.math.abs
import kotlin.math.roundToInt

/**
 * Layout checks for full-launch tests at a large system font scale: whether a control or text
 * scrolls into its window whole, isn't clipped or squeezed, lays out whole and takes a click
 *
 * UiAutomator reports bounds already clipped to the display, window and scroll container, and a
 * text's whole string even when it draws cut short, so every check reads the Compose semantics
 * tree of each window through [WindowInspector]. That covers a sheet or dialog, which is its own
 * window, without naming its texts
 */
class FullLaunchLayoutChecks(
    private val device: UiDevice,
    private val wallet: FullLaunchWalletRobot,
) {
    companion object {

        // well under a line of text at 2x, so only a squeezed or clipped node falls below it
        private const val MIN_NODE_HEIGHT_DP = 24

        // how far one drag moves a node cut off at the top or bottom, and how many it gets
        private const val NUDGE_DP = 96
        const val MAX_NUDGES = 6

        // Compose positions are floats that the accessibility bounds round
        private const val CLIP_TOLERANCE_PX = 1

        // enough of a text to tell which one a failure names
        private const val TEXT_IN_MESSAGE = 40

        // taps in gesture navigation's bottom strip go to the system, not the app
        private const val GESTURE_STRIP_DP = 48
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
    fun reachable(
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
            val viewport =
                try {
                    node.scrollViewport()
                } catch (_: StaleObjectException) {
                    device.waitFor(selector, timeoutMs = 5_000L).scrollViewport()
                }

            nudge(viewport ?: Rect(0, 0, device.displayWidth, device.displayHeight), towardsTop = !cutTop)
            node = device.waitFor(selector, timeoutMs = 5_000L)
        }

        error("$name never showed whole after $MAX_NUDGES drags: ${control ?: "no control found"}")
    }

    /** Where a control shows on screen after every clip, and where it would lie unclipped */
    data class ControlBounds(
        val shown: Rect,
        val whole: Rect,
    )

    /** The bounds of the control holding the Compose node that UiAutomator shows at [visible] */
    fun controlBounds(visible: Rect): ControlBounds? {
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
                    else -> texts.firstNotNullOfOrNull { it.textLayoutProblem() }
                }
        }

        problem?.let { error("$name doesn't draw whole: $it") }
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
     * Fails unless every Compose text in the window that shows [titleVisible] lays out whole
     *
     * A sheet is its own window, so this covers each of its texts, such as a screen's
     * descriptions, without naming them. Text fields are left out, since they scroll a long entry
     * inside themselves by design
     */
    fun assertWindowTextsDrawWhole(
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
                        node.textLayoutProblem()?.let { "\"${text.take(TEXT_IN_MESSAGE)}\": $it" }
                    }
        }

        assertTrue("$screen has texts that don't draw whole: $problems", problems.isEmpty())
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
    fun assertWindowTextsShowWhole(
        screen: String,
        titleVisible: Rect,
    ) {
        var root: RootForTest? = null
        var texts = emptyList<Pair<Int, TextIdentity>>()
        val claimed = mutableSetOf<Int>()
        var shared = emptySet<TextIdentity>()

        InstrumentationRegistry.getInstrumentation().runOnMainSync {
            root = semanticsNodeAt(titleVisible)?.first
            val nodes =
                root
                    ?.semanticsOwner
                    ?.getAllSemanticsNodes(mergingEnabled = false)
                    ?.filter { SemanticsProperties.Text in it.config && SemanticsProperties.EditableText !in it.config }
                    .orEmpty()

            // a replacement is a text made after the scan starts, so none of these can be one
            claimed += nodes.map { it.id }

            // a scroll can replace one of these with the other, so neither is told apart once it goes
            shared =
                nodes
                    .groupingBy { it.textIdentity() }
                    .eachCount()
                    .filterValues { it > 1 }
                    .keys
            texts =
                nodes
                    .filter { it.size.height > 0 && it.size.width > 0 }
                    .sortedByDescending { it.positionOnScreen.y }
                    .map { node -> node.id to node.textIdentity() }
        }

        val owner = checkNotNull(root) { "$screen: its title matches no Compose semantics node" }
        val problems =
            texts.mapNotNull { (id, identity) ->
                textShowWholeProblem(owner, id, identity, identity in shared, claimed)?.let { "\"${identity.text.take(TEXT_IN_MESSAGE)}\": $it" }
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
     * What tells a text from others that read the same: its string, and the test tag or the texts
     * of its nearest ancestor that has a test tag or takes a click, or null when none does
     */
    private data class TextIdentity(
        val text: String,
        val anchor: String?,
    )

    // read on the main thread, as every semantics read is
    private fun SemanticsNode.textIdentity(): TextIdentity {
        val anchor =
            generateSequence(this) { it.parent }.firstNotNullOfOrNull { node ->
                node.config.getOrNull(SemanticsProperties.TestTag)?.let { "tag $it" }
                    ?: node.takeIf { SemanticsActions.OnClick in it.config }?.let { control ->
                        "control " + control.selfAndDescendants().mapNotNull { it.config.getOrNull(SemanticsProperties.Text) }.flatten().joinToString(" ") { it.text }
                    }
            }

        return TextIdentity(config[SemanticsProperties.Text].joinToString(" ") { it.text }, anchor)
    }

    /** The text that took a missing text's place, or why none did */
    private sealed interface Replacement {
        data class Found(
            val id: Int,
        ) : Replacement

        data object Gone : Replacement

        data class Ambiguous(
            val count: Int,
        ) : Replacement
    }

    /**
     * Why the text with semantics [id] never shows whole, or null when it does
     *
     * Its scroll area is scrolled just far enough to bring the part under check inside it. A text
     * taller than the area must show its whole first line and then its whole last line, from the
     * line bounds its layout measures, so an area too short for a line fails
     *
     * A scroll can replace the text's subtree, which gives it a new semantics id, so a text that
     * goes missing is looked for again by its [identity] among the texts no other text in the scan
     * has [claimed], see [replacementText]. It fails when none or more than one is found, since
     * measuring a text that reads the same elsewhere would pass the missing one unchecked. For the
     * same reason a missing text whose identity another text [shared] when the scan began fails
     * outright: the same scroll can remove it and replace the other, which then looks like its
     * only replacement
     */
    private fun textShowWholeProblem(
        root: RootForTest,
        id: Int,
        identity: TextIdentity,
        shared: Boolean,
        claimed: MutableSet<Int>,
    ): String? {
        val edges = mutableListOf("whole")
        var checked = 0
        var scrolledFrom: Rect? = null
        var nodeId = id
        val gone = "it left its window: no new text there reads the same under the same anchor after $STALE_READS reads"

        repeat(MAX_NUDGES * 2) {
            val placement =
                textPlacement(root, nodeId) ?: run {
                    if (shared) {
                        return "it was replaced, and which text replaced it is ambiguous: another text there read the same under the same anchor when the check began"
                    }

                    when (val replacement = replacementText(root, identity, claimed)) {
                        is Replacement.Found -> {
                            nodeId = replacement.id
                            claimed += replacement.id
                            textPlacement(root, replacement.id) ?: return gone
                        }

                        Replacement.Gone -> return gone
                        is Replacement.Ambiguous ->
                            return "it was replaced, and which text replaced it is ambiguous: ${replacement.count} new texts there read the same under the same anchor after $STALE_READS reads"
                    }
                }

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

    /**
     * The one text in [root] with [identity] that no text in the scan has [claimed], read up to
     * [STALE_READS] times, each once the app is idle, until exactly one is found
     */
    private fun replacementText(
        root: RootForTest,
        identity: TextIdentity,
        claimed: Set<Int>,
    ): Replacement {
        var candidates = emptyList<Int>()

        repeat(STALE_READS) {
            device.waitForIdle()

            InstrumentationRegistry.getInstrumentation().runOnMainSync {
                candidates =
                    root.semanticsOwner
                        .getAllSemanticsNodes(mergingEnabled = false)
                        .filter { SemanticsProperties.Text in it.config && SemanticsProperties.EditableText !in it.config }
                        .filter { it.id !in claimed && it.textIdentity() == identity }
                        .map { it.id }
            }

            candidates.singleOrNull()?.let { return Replacement.Found(it) }
        }

        return if (candidates.isEmpty()) Replacement.Gone else Replacement.Ambiguous(candidates.size)
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

            // line tops and bottoms match the drawn text, as textLayoutProblem explains
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

    /**
     * Drags the screen's scroll area down, above any keyboard, until a match for [selector] shows,
     * for content above the top, which [reachable] never scrolls back to
     */
    fun scrollBackTo(
        selector: BySelector,
        name: String,
    ) {
        repeat(MAX_NUDGES * 3) {
            if (device.hasObject(selector)) return

            val viewport = largestScrollArea(name)
            val bottom = minOf(viewport.bottom, keyboardTop() ?: viewport.bottom)
            nudge(Rect(viewport.left, viewport.top, viewport.right, bottom), towardsTop = false)
        }

        assertTrue("$name never scrolled back into view", device.hasObject(selector))
    }

    /** The top of the keyboard on screen, or null while it is closed */
    fun keyboardTop(): Int? {
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

    /**
     * The bounds of the largest scroll area on screen, read again when a recomposition replaces a
     * node between finding it and reading it
     */
    fun largestScrollArea(name: String): Rect {
        repeat(STALE_READS) {
            try {
                return device
                    .findObjects(By.scrollable(true))
                    .map { it.visibleBounds }
                    .maxByOrNull { it.width() * it.height() }
                    ?: error("$name: the screen has no scroll area")
            } catch (_: StaleObjectException) {
                device.waitForIdle()
            }
        }

        error("$name: the scroll area kept changing under each read")
    }

    /**
     * Waits until the screen's content lies in the same place on two reads in a row, each taken
     * once the app is idle, and fails if it never does
     *
     * The first of [anchors] on screen marks the content, which is measured with the anchor's
     * nearest scrollable ancestor, so a screen behind a sheet doesn't count, or with the root of
     * the anchor's window when nothing scrolls, since Compose marks an area scrollable only while
     * its content overflows it
     */
    fun awaitContentAtRest(
        name: String,
        vararg anchors: BySelector,
    ) {
        var was: List<Rect>? = null

        wallet.waitUntil("$name never came to rest", timeoutMs = 15_000L) {
            device.waitForIdle()
            val content = contentBounds(anchors)
            (content != null && content == was).also { was = content }
        }
    }

    /**
     * Where the anchor, its container and each node directly inside that container lie, or null
     * when no anchor shows or a node went stale while it was read
     */
    private fun contentBounds(anchors: Array<out BySelector>): List<Rect>? =
        try {
            anchors.firstNotNullOfOrNull { device.findObject(it) }?.let { anchor ->
                val ancestors = generateSequence(anchor) { it.parent }.toList()
                val container = ancestors.firstOrNull { it.isScrollable } ?: ancestors.last()

                listOf(anchor.visibleBounds, container.visibleBounds) + container.children.map { it.visibleBounds }
            }
        } catch (_: StaleObjectException) {
            null
        }

    // the nearest scrollable ancestor's visible bounds, which is where a drag scrolls the node
    private fun UiObject2.scrollViewport(): Rect? =
        generateSequence(parent) { it.parent }
            .firstOrNull { it.isScrollable }
            ?.visibleBounds

    // a short slow drag, which moves the content without a fling
    fun nudge(
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
}

/** How many times a read retries after a recomposition replaces the node it read */
private const val STALE_READS = 5

/**
 * Sets the system font scale for a full-launch test and puts back the one the device had
 *
 * A sheet or dialog is its own window, which ignores a Compose test font scale, so a test of one
 * sets the system's. Call [save] before the test changes it and [restore] after it
 */
class SystemFontScale(
    private val device: UiDevice,
    private val wallet: FullLaunchWalletRobot,
) {
    private var previous: String? = null

    /** Records the device's font scale, which [restore] puts back */
    fun save() {
        previous = device.executeShellCommand("settings get system font_scale").trim()
    }

    /** Puts back the font scale [save] recorded, or the default when there was none */
    fun restore() {
        when (val previous = previous) {
            null, "null" -> device.executeShellCommand("settings delete system font_scale")
            else -> device.executeShellCommand("settings put system font_scale $previous")
        }
    }

    /**
     * Sets the largest system font scale and waits until the app's resumed activity has it
     *
     * The change recreates the activity, and a click that lands on the one it replaces is lost,
     * so the screen must be the new activity's before the test taps anything on it
     */
    fun setLargestFontScaleAndWaitForRecreation() {
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
}
