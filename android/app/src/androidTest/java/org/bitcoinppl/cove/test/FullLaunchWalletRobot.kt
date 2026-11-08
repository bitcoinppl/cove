package org.bitcoinppl.cove.test

import android.graphics.Rect
import android.view.WindowInsets
import androidx.test.platform.app.InstrumentationRegistry
import androidx.test.runner.lifecycle.ActivityLifecycleMonitorRegistry
import androidx.test.runner.lifecycle.Stage
import androidx.test.uiautomator.By
import androidx.test.uiautomator.BySelector
import androidx.test.uiautomator.StaleObjectException
import androidx.test.uiautomator.UiDevice
import androidx.test.uiautomator.UiObject2
import androidx.test.uiautomator.Until
import kotlinx.coroutines.runBlocking
import org.bitcoinppl.cove.AppManager
import org.bitcoinppl.cove.initialScanComplete
import org.bitcoinppl.cove_core.ApiType
import org.bitcoinppl.cove_core.AppAction
import org.bitcoinppl.cove_core.Database
import org.bitcoinppl.cove_core.NodeSelector
import org.bitcoinppl.cove_core.types.Network
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Assert.fail
import java.io.ByteArrayOutputStream
import java.math.BigDecimal

/**
 * BIP173 test vector: the P2WPKH address of the secp256k1 generator point's public key, which no
 * account of the abandon ×11 about mnemonic derives
 */
const val EXTERNAL_ADDRESS = "tb1qw508d6qejxtdg4y5r3zarvary0c5xw7kxpjzsx"

/**
 * Drives a funded wallet in the full app against cove-fake-node: network selection, balances and sends
 *
 * Uses UiAutomator rather than Compose test APIs, which stall a cold launch at bootstrap
 */
class FullLaunchWalletRobot(
    private val device: UiDevice,
) {
    // onboarding has no route to network or node settings, so this does in process what
    // Settings → Network and Settings → Node do
    fun selectSignetFakeNode() {
        val instrumentation = InstrumentationRegistry.getInstrumentation()
        var dispatched = false
        instrumentation.runOnMainSync {
            dispatched = AppManager.getInstance().dispatch(AppAction.ChangeNetwork(Network.SIGNET))
        }
        assertTrue("expected the network change to dispatch", dispatched)

        runBlocking {
            NodeSelector().use { selector ->
                val node = selector.parseCustomNode(FAKE_NODE_ESPLORA_URL, ApiType.ESPLORA, "Fake node")
                selector.checkAndSaveNode(node)
            }
        }

        Database().globalConfig().use { config ->
            assertEquals(Network.SIGNET, config.selectedNetwork())
            assertEquals(FAKE_NODE_ESPLORA_URL, config.selectedNode().url.trimEnd('/'))
        }
    }

    // coin control and send refuse to open until the initial full scan completes, which can finish
    // after the balance first shows
    fun waitForBalance(sats: Long) {
        val deadline = System.currentTimeMillis() + 90_000L
        var lastText: String? = null

        while (System.currentTimeMillis() < deadline) {
            dismissWalletSelectionSheetIfShown()

            // the balance renders only after bootstrap, before which AppManager can't be read
            val shown = device.findText(By.res("selectedWallet.balance"))
            if (shown != null) lastText = shown
            if (shown?.let(::parseSats) == sats && isInitialScanComplete()) return

            Thread.sleep(250)
        }

        error("Timed out waiting for a balance of $sats sats after the initial scan, last shown \"$lastText\"\n${device.hierarchy()}")
    }

    private fun isInitialScanComplete(): Boolean {
        var complete = false
        InstrumentationRegistry.getInstrumentation().runOnMainSync {
            complete = AppManager.getInstance().walletManager?.ledgerState?.initialScanComplete == true
        }

        return complete
    }

    // the import can offer other address types when they have history; the fake node gives them none,
    // but keep the native segwit wallet if the sheet shows anyway
    private fun dismissWalletSelectionSheetIfShown() {
        device.findObject(By.text("Keep Current"))?.click()
    }

    /** Opens Send from the wallet screen and enters the address */
    fun startSend(address: String = EXTERNAL_ADDRESS) {
        device.waitFor(By.text("Send")).click()
        enterAddress(address)
    }

    /**
     * Sets the address and waits until the screen shows it, setting it once more if it doesn't
     *
     * A second set replaces the field's whole text, so it can't enter the address twice
     */
    fun enterAddress(address: String = EXTERNAL_ADDRESS) {
        setAddress(address)
        if (showsAddressSoon(address)) return

        setAddress(address)
        if (!showsAddressSoon(address)) {
            fail("expected the address field to hold $address after setting it twice\n${device.hierarchy()}")
        }
    }

    // once the app accepts the address it moves focus to the amount, whose keyboard toolbar can
    // cover the spaced-out address; a covered node is left out of the accessibility tree, so close
    // the keyboard and look again before calling the address missing
    private fun showsAddressSoon(address: String): Boolean {
        if (becomesTrue(FIRST_ATTEMPT_TIMEOUT_MS) { showsAddress(address) }) return true

        dismissKeyboard()
        return becomesTrue(UNCOVERED_ADDRESS_TIMEOUT_MS) { showsAddress(address) }
    }

    private fun setAddress(address: String) {
        // the send screen opens with the amount focused, and the keyboard sliding in can move the
        // address field out from under a tap; tapping again is harmless once it has focus
        for (attempt in 1..3) {
            device.waitForIdle()
            device.scrollTo(By.res("sendFlow.address")).click()
            if (device.wait(Until.hasObject(By.res("sendFlow.address").focused(true)), 3_000L)) break
        }

        device.waitFor(By.res("sendFlow.address").focused(true))
        device.findObject(By.res("sendFlow.address")).text = address
    }

    // once the address is accepted the field can lose focus and show it in groups of four
    private fun showsAddress(address: String): Boolean =
        try {
            device.findObjects(By.textStartsWith(address.take(4))).any { node ->
                node.text?.filterNot { it.isWhitespace() } == address
            }
        } catch (_: StaleObjectException) {
            false
        }

    /** Fills the amount with Max and returns it in sats */
    fun tapMax(): Long {
        // Max lives in a toolbar shown only while the soft keyboard is up, which an emulator with a
        // hardware keyboard hides unless this is set. Tests keep it off until here because the
        // onboarding robot dismisses a visible keyboard with Back, which can race the import screen
        device.executeShellCommand("settings put secure show_ime_with_hard_keyboard 1")

        device.waitFor(By.res("sendFlow.amount")).click()
        device.waitFor(By.res("sendFlow.max")).click()
        waitUntil("expected Max to fill in the amount") {
            device.findText(By.res("sendFlow.amount"))?.any { it in '1'..'9' } == true
        }

        // the field shows the number alone, with the unit beside it; a BTC amount always has a point here
        val maxText = device.waitForText(By.res("sendFlow.amount")).orEmpty()
        val maxNumber = BigDecimal(maxText.filter { it.isDigit() || it == '.' })

        return if ('.' in maxText) maxNumber.movePointRight(8).longValueExact() else maxNumber.longValueExact()
    }

    /** Types an amount in the wallet's unit, BTC unless changed */
    fun enterAmount(amount: String) {
        typeInto(By.res("sendFlow.amount"), amount)
    }

    /**
     * Types into a field key by key, as the keyboard would. Cove's fields check each edit against
     * the last, and setting the whole text at once leaves them empty
     *
     * The emulator's keyboard can drop a key when its link to the field stalls, which leaves the
     * field a character short, so a field that doesn't hold [text] is cleared and typed once more.
     * [typed] still decides the result; the default needs the exact text
     */
    fun typeInto(
        field: BySelector,
        text: String,
        typed: () -> Boolean = { device.findText(field) == text },
    ) {
        typeKeys(field, text)
        if (becomesTrue(FIRST_ATTEMPT_TIMEOUT_MS, typed)) return

        clearByKeys(field)
        typeKeys(field, text)
        waitUntil("expected the field to hold $text after typing it twice", condition = typed)
    }

    private fun typeKeys(
        field: BySelector,
        text: String,
    ) {
        device.waitFor(field).click()
        device.waitForIdle()
        device.executeShellCommand("input text $text")
    }

    // selects everything and deletes it as one edit, as a keyboard would. Deleting key by key lets
    // the amount field reformat after each key, which can leave the cursor and its text out of step
    // with the keys that follow, and a line-end jump leaves earlier lines of a wrapped field behind.
    // The tap that focuses the field can land after the select-all and collapse it to a cursor, so
    // this checks the field emptied and selects again, without another tap, if it didn't
    private fun clearByKeys(field: BySelector) {
        device.waitFor(field).click()
        device.waitForIdle()

        repeat(CLEAR_ATTEMPTS) {
            device.executeShellCommand("input keycombination KEYCODE_CTRL_LEFT KEYCODE_A")
            device.executeShellCommand("input keyevent KEYCODE_DEL")
            if (becomesTrue(CLEAR_TIMEOUT_MS) { device.findText(field).isNullOrEmpty() }) return
        }
    }

    /** Taps Next and waits for the screen it leads to */
    fun clickNext(expecting: BySelector = By.res("sendFlowConfirm.swipeToSend")) {
        dismissKeyboard()
        device.scrollTo(By.res("sendFlow.next")).click()
        device.waitFor(expecting, timeoutMs = 30_000L)
    }

    /** Swipes the confirm screen's handle across, accepts the success alert and returns to the wallet */
    fun swipeToSend() {
        val track = device.waitFor(By.res("sendFlowConfirm.swipeToSend")).visibleBounds
        device.swipe(track.left + 40, track.centerY(), track.right - 4, track.centerY(), 40)

        device.waitFor(By.text("Transaction sent successfully!"), timeoutMs = 60_000L)
        device.waitFor(By.text("OK")).click()
        device.waitFor(By.res("selectedWallet.balance"), timeoutMs = 30_000L)
    }

    fun dismissKeyboard() {
        if (!isImeVisible()) return

        device.pressBack()
        waitUntil("expected the keyboard to close") { !isImeVisible() }
    }

    private fun isImeVisible(): Boolean {
        var visible = false
        InstrumentationRegistry.getInstrumentation().runOnMainSync {
            visible =
                ActivityLifecycleMonitorRegistry
                    .getInstance()
                    .getActivitiesInStage(Stage.RESUMED)
                    .firstOrNull()
                    ?.window
                    ?.decorView
                    ?.rootWindowInsets
                    ?.isVisible(WindowInsets.Type.ime()) == true
        }

        return visible
    }

    fun waitUntil(
        message: String,
        timeoutMs: Long = 20_000L,
        condition: () -> Boolean,
    ) {
        val deadline = System.currentTimeMillis() + timeoutMs

        while (System.currentTimeMillis() < deadline) {
            if (condition()) return

            Thread.sleep(100)
        }

        assertTrue("$message\n${device.hierarchy()}", condition())
    }

    // a first attempt that hasn't landed by now won't
    private fun becomesTrue(
        timeoutMs: Long,
        condition: () -> Boolean,
    ): Boolean {
        val deadline = System.currentTimeMillis() + timeoutMs

        while (System.currentTimeMillis() < deadline) {
            if (condition()) return true

            Thread.sleep(100)
        }

        return condition()
    }

    private companion object {
        const val FIRST_ATTEMPT_TIMEOUT_MS = 10_000L
        const val UNCOVERED_ADDRESS_TIMEOUT_MS = 5_000L
        const val CLEAR_ATTEMPTS = 3
        const val CLEAR_TIMEOUT_MS = 2_000L
    }
}

/** Parses an amount label such as `190,000 SATS` or `0.0019 BTC` into sats */
fun parseSats(text: String): Long? {
    val digits = text.filter { it.isDigit() || it == '.' }
    if (digits.isEmpty()) return null

    return when {
        text.contains("BTC", ignoreCase = true) -> BigDecimal(digits).movePointRight(8).longValueExact()
        text.contains("SAT", ignoreCase = true) -> digits.toLongOrNull()
        else -> null
    }
}

fun UiDevice.waitFor(
    selector: BySelector,
    timeoutMs: Long = 20_000L,
): UiObject2 = wait(Until.findObject(selector), timeoutMs) ?: error("Timed out waiting for $selector\n${hierarchy()}")

fun UiDevice.scrollTo(
    selector: BySelector,
    timeoutMs: Long = 20_000L,
): UiObject2 {
    val deadline = System.currentTimeMillis() + timeoutMs

    while (System.currentTimeMillis() < deadline) {
        findObject(selector)?.let { node ->
            if (isTappable(node)) {
                settledObject(selector)?.takeIf { isTappable(it) }?.let { return it }
            }
        }

        swipe(displayWidth / 2, displayHeight * 2 / 3, displayWidth / 2, displayHeight / 3, 20)
        Thread.sleep(250)
    }

    error("Timed out scrolling to $selector\n${hierarchy()}")
}

/** Whether a tap on the node reaches the app, which it doesn't in the 48dp gesture navigation strip */
fun UiDevice.isTappable(node: UiObject2): Boolean {
    val density = InstrumentationRegistry.getInstrumentation().targetContext.resources.displayMetrics.density

    return node.visibleBounds.bottom <= displayHeight - (48 * density).toInt()
}

// a swipe keeps flinging after it ends, and a tap on a moving node lands on whatever scrolls under it
private fun UiDevice.settledObject(selector: BySelector): UiObject2? {
    var lastBounds: Rect? = null

    repeat(10) {
        val node = findObject(selector) ?: return null
        val bounds =
            try {
                node.visibleBounds
            } catch (_: StaleObjectException) {
                null
            }

        if (bounds != null && bounds == lastBounds) return node

        lastBounds = bounds
        Thread.sleep(200)
    }

    return null
}

// a tag on a layout wrapper puts the text on a child node
fun UiObject2.anyText(): String? = text ?: children.firstNotNullOfOrNull { it.anyText() }

// a recomposition can replace a node between finding and reading it, which UiAutomator reports
// as stale, so these find the node again instead of failing on a slow device
fun UiDevice.findText(selector: BySelector): String? =
    try {
        findObject(selector)?.anyText()
    } catch (_: StaleObjectException) {
        null
    }

fun UiDevice.waitForText(selector: BySelector): String? {
    repeat(STALE_READ_ATTEMPTS - 1) {
        try {
            return waitFor(selector).anyText()
        } catch (_: StaleObjectException) {
            Thread.sleep(100)
        }
    }

    return waitFor(selector).anyText()
}

fun UiDevice.hierarchy(): String =
    ByteArrayOutputStream().use { output ->
        dumpWindowHierarchy(output)
        output.toString(Charsets.UTF_8.name()).take(120_000)
    }

private const val STALE_READ_ATTEMPTS = 5
