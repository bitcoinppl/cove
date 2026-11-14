package org.bitcoinppl.cove.flows.cloudbackup

import android.graphics.Rect
import android.os.Build
import androidx.test.platform.app.InstrumentationRegistry
import androidx.test.uiautomator.Configurator
import androidx.test.uiautomator.UiDevice
import org.w3c.dom.Element
import java.io.ByteArrayInputStream
import java.io.ByteArrayOutputStream
import javax.xml.parsers.DocumentBuilderFactory

// Cloud Backup full-launch tests read and tap the screen through snapshots rather than UiObject2.
// A UiObject2 waits for idle and then refreshes its node on every read, so a read throws
// StaleObjectException once a recomposition has replaced the node it found, which the detail does
// to its rows and Restore All whenever a progress section appears or goes. A snapshot is a copy of
// every window taken once the app is idle, so each read looks its target up again and sees the
// replacement, and a snapshot never goes stale. The dump would otherwise walk UiAutomation's node
// cache, which Compose invalidates only once its batched accessibility events arrive, so each
// snapshot clears that cache first and fetches every node live, as a UiObject2 read does

/** What a test reads or taps: a node's test tag, or the exact text it shows */
internal sealed interface UiTarget {
    val description: String

    data class Tag(
        val tag: String,
    ) : UiTarget {
        override val description: String get() = "tag \"$tag\""
    }

    data class Text(
        val text: String,
    ) : UiTarget {
        override val description: String get() = "text \"$text\""
    }

    /** Any text holding [text], for nodes such as a wallet row that merge their texts into one */
    data class TextContaining(
        val text: String,
    ) : UiTarget {
        override val description: String get() = "text containing \"$text\""
    }
}

/** A control as one snapshot showed it: whether it was enabled, and where it was on screen */
internal data class UiControl(
    val enabled: Boolean,
    val bounds: Rect,
)

/** The visible nodes of every window at one moment, in window and then document order */
internal class UiSnapshot private constructor(
    private val nodes: List<Element>,
) {
    /**
     * The clickable node holding the first visible match of [target], or null when no match sits
     * in something clickable
     *
     * A disabled Compose clickable stays clickable in the accessibility tree and reports enabled
     * false, and a text can also show as a plain label, such as a section header above its row
     */
    fun control(target: UiTarget): UiControl? =
        nodes
            .asSequence()
            .filter { node -> target.matches(node) }
            .firstNotNullOfOrNull { node -> node.selfAndAncestors().firstOrNull { it.flag(CLICKABLE) } }
            ?.let { clickable -> UiControl(enabled = clickable.flag(ENABLED), bounds = clickable.bounds()) }
            ?.takeUnless { control -> control.bounds.isEmpty }

    /** Whether any visible node matches [target] */
    fun shows(target: UiTarget): Boolean = nodes.any { node -> target.matches(node) }

    /** Every non-empty text on screen, in order */
    fun texts(): List<String> = nodes.mapNotNull { node -> node.getAttribute(TEXT).ifEmpty { null } }

    /** Every non-empty text on screen with where it is, so two snapshots tell whether anything moved */
    fun placedTexts(): List<Pair<String, Rect>> =
        nodes.mapNotNull { node -> node.getAttribute(TEXT).ifEmpty { null }?.let { it to node.bounds() } }

    /** The bounds of the first node marked scrollable, which Compose does only while it can scroll */
    fun scrollableBounds(): Rect? = nodes.firstOrNull { it.flag(SCROLLABLE) }?.bounds()

    /**
     * The innermost container of [target]'s first match that also shows [holding], as its own
     * snapshot, or null when no ancestor holds both
     */
    fun containerOf(
        target: UiTarget,
        holding: UiTarget,
    ): UiSnapshot? {
        val match = nodes.firstOrNull { node -> target.matches(node) } ?: return null

        return match
            .selfAndAncestors()
            .map { container -> UiSnapshot(container.descendantNodes()) }
            .firstOrNull { container -> container.shows(holding) }
    }

    companion object {
        private const val TEXT = "text"
        private const val RESOURCE_ID = "resource-id"
        private const val CLICKABLE = "clickable"
        private const val ENABLED = "enabled"
        private const val SCROLLABLE = "scrollable"
        private const val NODE = "node"
        private val BOUNDS = Regex("""\[(-?\d+),(-?\d+)]\[(-?\d+),(-?\d+)]""")

        /** Parses a UiAutomator window dump, which holds only the nodes visible to the user */
        fun parse(xml: ByteArray): UiSnapshot {
            val document = DocumentBuilderFactory.newInstance().newDocumentBuilder().parse(ByteArrayInputStream(xml))
            val nodes = document.getElementsByTagName(NODE)

            return UiSnapshot((0 until nodes.length).map { index -> nodes.item(index) as Element })
        }

        private fun UiTarget.matches(node: Element): Boolean =
            when (this) {
                is UiTarget.Tag -> node.getAttribute(RESOURCE_ID) == tag
                is UiTarget.Text -> node.getAttribute(TEXT) == text
                is UiTarget.TextContaining -> text in node.getAttribute(TEXT)
            }

        private fun Element.flag(name: String): Boolean = getAttribute(name) == "true"

        private fun Element.selfAndAncestors(): Sequence<Element> =
            generateSequence(this) { node -> (node.parentNode as? Element)?.takeIf { it.tagName == NODE } }

        private fun Element.descendantNodes(): List<Element> {
            val descendants = getElementsByTagName(NODE)

            return listOf(this) + (0 until descendants.length).map { index -> descendants.item(index) as Element }
        }

        private fun Element.bounds(): Rect {
            val (left, top, right, bottom) =
                BOUNDS.matchEntire(getAttribute("bounds"))?.destructured
                    ?: return Rect()

            return Rect(left.toInt(), top.toInt(), right.toInt(), bottom.toInt())
        }
    }
}

/** A copy of every window's visible nodes, taken once the app is idle */
internal fun UiDevice.snapshot(): UiSnapshot {
    // a cached dump taken straight after a state change can still show the screen from before it
    clearNodeCache()

    val xml =
        ByteArrayOutputStream().use { output ->
            dumpWindowHierarchy(output)
            output.toByteArray()
        }

    return UiSnapshot.parse(xml)
}

// the same connected UiAutomation UiDevice uses, since asking with other flags would reconnect it;
// API 33, the lowest this app supports, has no way to clear the cache, so its dumps can lag
private fun clearNodeCache() {
    if (Build.VERSION.SDK_INT < Build.VERSION_CODES.UPSIDE_DOWN_CAKE) return

    InstrumentationRegistry
        .getInstrumentation()
        .getUiAutomation(Configurator.getInstance().uiAutomationFlags)
        .clearCache()
}

/** Whether the control holding [target] is enabled right now, or null when none is on screen */
internal fun UiDevice.controlEnabled(target: UiTarget): Boolean? = snapshot().control(target)?.enabled

/** Waits until [target] shows inside something clickable, and returns that control as it was then */
internal fun UiDevice.waitForControl(
    target: UiTarget,
    timeoutMs: Long = 20_000L,
): UiControl {
    val deadline = System.currentTimeMillis() + timeoutMs

    while (System.currentTimeMillis() < deadline) {
        snapshot().control(target)?.let { return it }

        Thread.sleep(POLL_MS)
    }

    error("Timed out waiting for a control with ${target.description}; on screen: ${snapshot().texts()}")
}

/**
 * Waits until the control holding [target] held still between two snapshots, so a tap lands where
 * it was found rather than where a sliding sheet or dialog has moved it
 */
internal fun UiDevice.waitForSettledControl(
    target: UiTarget,
    timeoutMs: Long = 20_000L,
): UiControl {
    val deadline = System.currentTimeMillis() + timeoutMs
    var previous: Rect? = null

    while (System.currentTimeMillis() < deadline) {
        val control = snapshot().control(target)
        if (control != null && control.bounds == previous) return control

        previous = control?.bounds
        Thread.sleep(SETTLE_POLL_MS)
    }

    error("A control with ${target.description} never held still, last bounds $previous; on screen: ${snapshot().texts()}")
}

/**
 * Taps the middle of the control holding [target] once it holds still, whether or not it is
 * enabled, since tests also tap disabled controls to show they do nothing
 */
internal fun UiDevice.tap(
    target: UiTarget,
    timeoutMs: Long = 20_000L,
) {
    val bounds = waitForSettledControl(target, timeoutMs).bounds

    click(bounds.centerX(), bounds.centerY())
}

/**
 * Swipes until [target] is on screen, [swipesPerDirection] swipes one way and then as many the
 * other in turn, since the detail may already be scrolled past it; [upwardFirst] starts toward the
 * top, and [message] leads the failure when it never shows
 */
internal fun UiDevice.scrollUntilShown(
    target: UiTarget,
    timeoutMs: Long = 20_000L,
    upwardFirst: Boolean = false,
    message: String = "Timed out scrolling to ${target.description}",
    swipesPerDirection: Int = 4,
) {
    val deadline = System.currentTimeMillis() + timeoutMs
    var swipes = 0

    while (System.currentTimeMillis() < deadline) {
        if (snapshot().shows(target)) return

        val upward = ((swipes / swipesPerDirection) % 2 == 1) != upwardFirst
        val startY = if (upward) displayHeight / 4 else displayHeight * 3 / 4
        val endY = if (upward) displayHeight * 3 / 4 else displayHeight / 4
        swipe(displayWidth / 2, startY, displayWidth / 2, endY, 20)
        swipes += 1

        // a tap while the list still flings only stops it, so the next look waits for it to settle
        waitForIdle()
    }

    error("$message; on screen: ${snapshot().texts()}")
}

/** Scrolls until [target] is on screen, then taps its control once it holds still */
internal fun UiDevice.scrollAndTap(
    target: UiTarget,
    timeoutMs: Long = 20_000L,
    upwardFirst: Boolean = false,
) {
    scrollUntilShown(target, timeoutMs, upwardFirst)
    tap(target, timeoutMs)
}

private const val POLL_MS = 100L
private const val SETTLE_POLL_MS = 200L
