package org.bitcoinppl.cove.flows.settings

import android.graphics.Rect
import androidx.test.uiautomator.By
import androidx.test.uiautomator.BySelector
import androidx.test.uiautomator.StaleObjectException
import androidx.test.uiautomator.UiDevice
import androidx.test.uiautomator.UiObject2
import org.bitcoinppl.cove.test.hierarchy
import org.bitcoinppl.cove.test.waitFor

// the switch is the row's trailing content, not a child of its title, so it is the checkable node
// laid out level with the title
internal fun UiDevice.switchInRowOf(title: String): UiObject2 {
    val titleBounds = waitForSettled(By.text(title), "the \"$title\" row").visibleBounds

    return findObjects(By.checkable(true)).singleOrNull { it.visibleCenter.y in titleBounds.top..titleBounds.bottom }
        ?: error("no single switch level with \"$title\"\n${hierarchy()}")
}

internal fun UiDevice.clickSwitchInRowOf(title: String) {
    switchInRowOf(title).click()
}

// the decoy PIN pad slides up as a sheet, and a key found mid-slide sends the tap to its neighbour,
// so the bottom row must hold still first. The pad checks a full PIN after a short delay, which the
// callers wait out by waiting for the next screen
internal fun UiDevice.enterPin(pin: String) {
    waitForSettled(pinKey('0'), "the number pad")

    pin.forEach { digit -> waitFor(pinKey(digit), timeoutMs = 5_000L).click() }
}

// the sheet is a dialog window of its own, which doesn't expose the pad's test tags as resource
// ids, so a key is the clickable node holding its digit
internal fun pinKey(digit: Char): BySelector = By.clickable(true).hasChild(By.text(digit.toString()))

internal fun UiDevice.clickText(value: String) {
    clickSettled(By.text(value), "text \"$value\"")
}

internal fun UiDevice.clickSettled(
    selector: BySelector,
    description: String,
) {
    waitFor(selector)
    var clickable: UiObject2? = waitForSettled(selector, description)
    while (clickable != null && !clickable.isClickable) {
        clickable = clickable.parent
    }

    requireNotNull(clickable) { "expected a clickable ancestor for $description" }.click()
}

/**
 * Waits until [selector] matches a node whose bounds held still between two looks, so a tap on
 * it lands where it was found rather than where a sliding screen has moved it
 */
internal fun UiDevice.waitForSettled(
    selector: BySelector,
    description: String,
    timeoutMs: Long = 20_000L,
): UiObject2 {
    val deadline = System.currentTimeMillis() + timeoutMs
    var previous: Rect? = null

    while (System.currentTimeMillis() < deadline) {
        val node = findObject(selector)
        val bounds =
            try {
                node?.visibleBounds
            } catch (_: StaleObjectException) {
                null
            }

        if (node != null && bounds != null && bounds == previous) return node

        previous = bounds
        Thread.sleep(SETTLE_POLL_MS)
    }

    error("$description never held still, last bounds $previous\n${hierarchy()}")
}

private const val SETTLE_POLL_MS = 200L
