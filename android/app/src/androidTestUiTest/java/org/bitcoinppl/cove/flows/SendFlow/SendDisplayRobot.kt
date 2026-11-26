package org.bitcoinppl.cove.flows.SendFlow

import android.system.Os
import androidx.test.platform.app.InstrumentationRegistry
import androidx.test.uiautomator.By
import androidx.test.uiautomator.BySelector
import androidx.test.uiautomator.StaleObjectException
import androidx.test.uiautomator.UiDevice
import androidx.test.uiautomator.UiObject2
import androidx.test.uiautomator.Until
import org.bitcoinppl.cove.AppManager
import org.bitcoinppl.cove.AppSheetState
import org.bitcoinppl.cove.test.EXTERNAL_ADDRESS
import org.bitcoinppl.cove.test.FAKE_NODE_ESPLORA_URL
import org.bitcoinppl.cove.test.FakeNodeBroadcast
import org.bitcoinppl.cove.test.FakeNodeControl
import org.bitcoinppl.cove.test.FakeNodeFundedOutput
import org.bitcoinppl.cove.test.FullLaunchOnboardingRobot
import org.bitcoinppl.cove.test.FullLaunchStartupRobot
import org.bitcoinppl.cove.test.FullLaunchWalletRobot
import org.bitcoinppl.cove.test.QrScript
import org.bitcoinppl.cove.test.TEST_MNEMONIC
import org.bitcoinppl.cove.test.findText
import org.bitcoinppl.cove.test.hierarchy
import org.bitcoinppl.cove.test.launchFullApp
import org.bitcoinppl.cove.test.parseSats
import org.bitcoinppl.cove.test.scrollTo
import org.bitcoinppl.cove.test.waitFor
import org.junit.Assert.assertEquals
import org.junit.Assert.fail
import java.math.BigDecimal
import kotlin.math.abs

/** How the wallet screen shows its balance, which also sets the unit and entry mode Send opens in */
enum class BalanceDisplay {
    BTC,
    FIAT_OVER_BTC,
    SATS,
    FIAT_OVER_SATS,
}

/**
 * Drives the Send screen's amount, unit, fiat entry and confirm summary against cove-fake-node,
 * which prices bitcoin at 100,000 per BTC in every currency through `COVE_PRICE_URL`, so 1,000
 * sats is 1.00 in fiat
 */
class SendDisplayRobot(
    private val device: UiDevice,
) {
    val wallet = FullLaunchWalletRobot(device)
    val node = FakeNodeControl()

    /** Points fee and price fetches at the fake node; debug builds read both on every fetch */
    fun useFakeNodeFeesAndPrices() {
        Os.setenv("COVE_FEE_URL", "$FAKE_NODE_ESPLORA_URL/api/v1/fees/recommended", true)
        Os.setenv("COVE_PRICE_URL", "$FAKE_NODE_ESPLORA_URL/api/v1/prices", true)
    }

    /** Funds one coin of each amount, imports the test wallet and waits for its first full scan */
    fun launchFundedWallet(coins: List<Long>): List<FakeNodeFundedOutput> {
        node.reset()
        val funded = node.fundWallet(TEST_MNEMONIC, coins.mapIndexed { index, sats -> index to sats })
        node.mine()

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
        wallet.waitForBalance(coins.sum())

        return funded
    }

    // MARK: wallet screen

    /**
     * Taps the balance until it shows [display]; each tap steps through btc, fiat over btc, sats
     * and fiat over sats
     */
    fun showBalanceAs(display: BalanceDisplay) {
        repeat(BalanceDisplay.entries.size) {
            val before = waitForKnownBalanceDisplay()
            if (before == display) return

            val shown = balanceTexts()
            device.waitFor(By.res(BALANCE_TOGGLE_TAG)).click()
            wallet.waitUntil("the balance didn't change after tapping it, still $shown", timeoutMs = 10_000L) {
                balanceTexts() != shown
            }
        }

        assertEquals("the balance never showed $display", display, waitForKnownBalanceDisplay())
    }

    // a fiat balance shows a spinner until prices arrive, so wait until the display can be told apart
    private fun waitForKnownBalanceDisplay(): BalanceDisplay {
        var display: BalanceDisplay? = null
        waitShowing("the balance display never settled", { balanceTexts().toString() }, timeoutMs = 30_000L) {
            display = balanceDisplay()
            display != null
        }

        return checkNotNull(display)
    }

    private fun balanceDisplay(): BalanceDisplay? {
        val primary = device.findText(By.res(BALANCE_TAG))?.trim() ?: return null
        if (primary.endsWith(" BTC")) return BalanceDisplay.BTC
        if (primary.endsWith(" SATS")) return BalanceDisplay.SATS
        if (!primary.any { it.isDigit() }) return null

        val secondary = balanceTexts().filterNot { it.trim() == primary }
        return when {
            secondary.any { it.trim().endsWith(" SATS") } -> BalanceDisplay.FIAT_OVER_SATS
            secondary.any { it.trim().endsWith(" BTC") } -> BalanceDisplay.FIAT_OVER_BTC
            else -> null
        }
    }

    private fun balanceTexts(): List<String> = device.textsUnder(By.res(BALANCE_TOGGLE_TAG))

    /** Opens the wallet screen's scanner and shows it [code], as if held up to the camera */
    fun scanFromWalletScreen(
        qr: QrScript,
        code: String,
    ) {
        repeat(3) {
            device.waitFor(By.desc("QR Code")).click()
            if (becomesTrue(5_000L) { readOnMain { AppManager.getInstance().sheetState?.item == AppSheetState.Qr } }) {
                qr.show(listOf(code))
                return
            }
        }

        error("the wallet screen's scanner never opened\n${device.hierarchy()}")
    }

    // MARK: send screen

    /** The amount field's text as shown, with grouping commas and any currency symbol */
    fun amountText(): String? = device.findText(By.res(AMOUNT_TAG))

    fun waitForUnit(unit: String) {
        wallet.waitUntil("expected the send amount unit to read $unit", timeoutMs = 10_000L) {
            device.findText(By.res(UNIT_TAG)) == unit
        }
    }

    fun assertNoUnitSelector(message: String) {
        wallet.waitUntil(message, timeoutMs = 10_000L) { !device.hasObject(By.res(UNIT_TAG)) }
    }

    fun selectUnit(unit: String) {
        wallet.dismissKeyboard()
        device.waitFor(By.res(UNIT_TAG)).click()
        device.waitFor(By.res("$UNIT_TAG.$unit"), timeoutMs = 5_000L).click()
        waitForUnit(unit)
    }

    /** Types a bitcoin amount and waits for the field to hold it, ignoring grouping commas */
    fun typeAmount(amount: String) {
        wallet.typeInto(By.res(AMOUNT_TAG), amount) { amountText()?.replace(",", "") == amount }
    }

    /** Clears the amount field as a keyboard would, then types [amount] in its place */
    fun replaceAmount(
        amount: String,
        typed: () -> Boolean,
    ) {
        clearAmount()
        wallet.typeInto(By.res(AMOUNT_TAG), amount, typed)
    }

    // select all and delete in one edit; see FullLaunchWalletRobot.clearByKeys
    private fun clearAmount() {
        device.waitFor(By.res(AMOUNT_TAG)).click()
        device.waitForIdle()

        repeat(3) {
            device.executeShellCommand("input keycombination KEYCODE_CTRL_LEFT KEYCODE_A")
            device.executeShellCommand("input keyevent KEYCODE_DEL")
            if (becomesTrue(2_000L) { amountText().orEmpty().none { it.isDigit() } }) return
        }

        error("the amount field never cleared, still \"${amountText()}\"\n${device.hierarchy()}")
    }

    /** Types [keys] after what the field already holds */
    fun appendToAmount(keys: String) {
        device.waitFor(By.res(AMOUNT_TAG)).click()
        device.waitForIdle()
        device.executeShellCommand("input keyevent KEYCODE_MOVE_END")
        device.executeShellCommand("input text $keys")
    }

    fun waitForAmountText(
        text: String,
        message: String,
    ) {
        waitShowing(message, ::amountText) { amountText() == text }
    }

    /** Waits for the bitcoin amount field to show [text], ignoring grouping commas */
    fun waitForBitcoinAmount(
        text: String,
        message: String,
    ) {
        waitShowing(message, ::amountText) { amountText()?.replace(",", "") == text }
    }

    /** Waits for the fiat amount field to show [amount] after its currency symbol, with or without cents */
    fun waitForFiatAmount(
        amount: String,
        message: String,
    ) {
        waitShowing(message, ::amountText) { fiatDigits(amountText()) in setOf(amount, "$amount.00") }
    }

    /** The row under the amount: fiat while entering bitcoin, bitcoin and its unit while entering fiat */
    fun secondaryText(): String = device.textsUnder(By.res(SECONDARY_TAG)).joinToString(" ")

    fun waitForSecondary(
        containing: String,
        message: String,
    ) {
        waitShowing(message, ::secondaryText, timeoutMs = 30_000L) { containing in secondaryText() }
    }

    fun tapSecondary() {
        device.waitFor(By.res(SECONDARY_TAG)).click()
    }

    fun tapHeaderVisibility() {
        device.waitFor(By.res(HEADER_VISIBILITY_TAG)).click()
    }

    /** Taps Max and returns the amount field's text once Max has filled it */
    fun tapMax(): String {
        // Max lives in a toolbar on the soft keyboard; see FullLaunchWalletRobot.tapMax
        device.executeShellCommand("settings put secure show_ime_with_hard_keyboard 1")

        device.waitFor(By.res(AMOUNT_TAG)).click()
        device.waitFor(By.res(MAX_TAG)).click()
        wallet.waitUntil("expected Max to fill in the amount") {
            amountText()?.any { it in '1'..'9' } == true
        }

        // the drain can land after the first digits show, so wait for the text to settle
        var settled = amountText()
        waitShowing("the Max amount never settled", { settled }) {
            Thread.sleep(500)
            val now = amountText()
            (now == settled).also { settled = now }
        }

        return settled.orEmpty()
    }

    // MARK: coin control

    fun openManageUtxos() {
        device.waitFor(By.desc("More")).click()
        device.waitFor(By.text("More Options"))

        // the sheet animates open half expanded; see UtxoLockFullLaunchTest
        repeat(3) {
            device.waitForIdle()
            device.findObject(By.res("selectedWallet.manageUtxos"))?.click()
                ?: device.scrollTo(By.res("selectedWallet.manageUtxos")).click()
            if (device.wait(Until.hasObject(By.res("coinControl.selectAll")), 5_000L)) return
        }

        device.waitFor(By.res("coinControl.selectAll"))
    }

    fun selectAllCoins() {
        device.waitFor(By.res("coinControl.selectAll").text("Select All"), timeoutMs = 30_000L).click()
        device.waitFor(By.res("coinControl.selectAll").text("Deselect All"))
    }

    fun continueToCoinControlAmount() {
        device.waitFor(By.res("coinControl.continue")).click()
        device.waitFor(By.res(COIN_CONTROL_AMOUNT_TAG), timeoutMs = 30_000L)
    }

    fun coinControlAmountText(): String? = device.findText(By.res(COIN_CONTROL_AMOUNT_TAG))

    fun coinControlFiatText(): String? = device.findText(By.res(COIN_CONTROL_FIAT_TAG))

    /** Waits for coin control's amount to settle on a number and returns it without grouping commas */
    fun waitForCoinControlAmount(message: String): String {
        var last: String? = null
        waitShowing(message, { last }, timeoutMs = 20_000L) {
            Thread.sleep(300)
            val now = coinControlAmountText()?.replace(",", "")
            val stable = now != null && now.any { it in '1'..'9' } && now == last
            last = now
            stable
        }

        return checkNotNull(last)
    }

    /** Waits for coin control's fiat line to show [sats] in fiat, to the cent either way it rounds */
    fun waitForCoinControlFiatNear(
        sats: Long,
        message: String,
    ) {
        waitShowing(message, ::coinControlFiatText, timeoutMs = 30_000L) {
            fiatNear(coinControlFiatText(), sats)
        }
    }

    fun openCustomAmountSheet() {
        wallet.dismissKeyboard()
        device.waitFor(By.res(COIN_CONTROL_AMOUNT_TAG)).click()
        device.waitFor(By.res(CUSTOM_AMOUNT_TAG), timeoutMs = 10_000L)
    }

    fun customAmountText(): String? = device.findText(By.res(CUSTOM_AMOUNT_TAG))

    fun closeCustomAmountSheet() {
        wallet.dismissKeyboard()
        device.waitFor(By.desc("Close")).click()
        wallet.waitUntil("the custom amount sheet didn't close") { !device.hasObject(By.res(CUSTOM_AMOUNT_TAG)) }
    }

    // MARK: confirm screen

    /** The value of a confirm screen summary row, such as `sendFlowConfirm.fee` */
    fun confirmValue(tag: String): String {
        var value: String? = null
        wallet.waitUntil("the confirm screen's $tag never showed an amount") {
            value = device.textsUnder(By.res(tag)).lastOrNull { text -> text.any { it.isDigit() } }
            value != null
        }

        return checkNotNull(value)
    }

    // MARK: broadcasts

    fun singleBroadcast(): FakeNodeBroadcast {
        val broadcasts = node.broadcasts()
        assertEquals("expected exactly one broadcast", 1, broadcasts.size)

        return broadcasts.single()
    }

    /** Waits for [condition], and on a timeout fails with what [actual] reads then */
    private fun waitShowing(
        message: String,
        actual: () -> String?,
        timeoutMs: Long = 10_000L,
        condition: () -> Boolean,
    ) {
        if (becomesTrue(timeoutMs, condition)) return

        fail("$message, got \"${actual()}\"\n${device.hierarchy()}")
    }

    private fun <T> readOnMain(read: () -> T): T {
        var value: T? = null
        InstrumentationRegistry.getInstrumentation().runOnMainSync { value = read() }

        @Suppress("UNCHECKED_CAST")
        return value as T
    }

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

    companion object {
        const val BALANCE_TAG = "selectedWallet.balance"
        const val BALANCE_TOGGLE_TAG = "selectedWallet.balanceUnitToggle"
        const val AMOUNT_TAG = "sendFlow.amount"
        const val UNIT_TAG = "sendFlow.unit"
        const val MAX_TAG = "sendFlow.max"
        const val SECONDARY_TAG = "sendFlow.secondaryAmount"
        const val HEADER_VISIBILITY_TAG = "sendFlow.header.visibility"
        const val COIN_CONTROL_AMOUNT_TAG = "sendFlow.coinControl.amount"
        const val COIN_CONTROL_FIAT_TAG = "sendFlow.coinControl.fiatAmount"
        const val CUSTOM_AMOUNT_TAG = "sendFlow.coinControl.customAmount"
        const val FEE_TAG = "sendFlowConfirm.fee"
        const val TOTAL_TAG = "sendFlowConfirm.total"

        /** What Rust shows in place of a fiat amount while amounts are hidden */
        const val MASKED_AMOUNT = "**************"
    }
}

/** The digits and point of a fiat amount, dropping its currency symbol and grouping */
fun fiatDigits(text: String?): String = text.orEmpty().filter { it.isDigit() || it == '.' }

/** A fiat amount shown to the cent as sats at the fake node's price */
fun fiatAsSats(text: String?): Long = BigDecimal(fiatDigits(text)).movePointRight(3).longValueExact()

/** Whether [text] shows [sats] in fiat, to the cent whichever way it rounds; a cent is 10 sats */
fun fiatNear(
    text: String?,
    sats: Long,
): Boolean {
    val digits = fiatDigits(text)
    if (digits.isEmpty() || digits.count { it == '.' } > 1) return false

    return abs(fiatAsSats(digits) - sats) < 10L
}

/** Parses an amount in [unit], `btc` or `sats`, into sats */
fun bitcoinAmountSats(
    text: String,
    unit: String,
): Long {
    val digits = text.filter { it.isDigit() || it == '.' }

    return when (unit) {
        "btc" -> BigDecimal(digits).movePointRight(8).longValueExact()
        else -> digits.toLong()
    }
}

fun FakeNodeBroadcast.sentToExternal(): Long = outputs.single { it.address == EXTERNAL_ADDRESS }.sats

/** The fee a broadcast paid, from the funded coins it spent */
fun FakeNodeBroadcast.fee(funded: List<FakeNodeFundedOutput>): Long {
    val spent = inputs.sumOf { input -> funded.single { it.outpoint == input }.sats }

    return spent - outputs.sumOf { it.sats }
}

/** A confirm screen amount such as `12,000 SATS` in sats */
fun confirmSats(text: String): Long = checkNotNull(parseSats(text)) { "\"$text\" isn't an amount in btc or sats" }

// a tag on a layout wrapper, or a clickable row that merges its children, can hold the text on the
// node itself or on its children, so this reads both
private fun UiObject2.allTexts(): List<String> = listOfNotNull(text) + children.flatMap { it.allTexts() }

fun UiDevice.textsUnder(selector: BySelector): List<String> =
    try {
        findObject(selector)?.allTexts()?.distinct().orEmpty()
    } catch (_: StaleObjectException) {
        emptyList()
    }
