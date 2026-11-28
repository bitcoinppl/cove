package org.bitcoinppl.cove.flows.SendFlow

import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.filters.LargeTest
import androidx.test.uiautomator.By
import androidx.test.uiautomator.UiDevice
import org.bitcoinppl.cove.flows.SendFlow.SendDisplayRobot.Companion.FEE_TAG
import org.bitcoinppl.cove.flows.SendFlow.SendDisplayRobot.Companion.TOTAL_TAG
import org.bitcoinppl.cove.test.AndroidDeviceStayAwakeRule
import org.bitcoinppl.cove.test.EXTERNAL_ADDRESS
import org.bitcoinppl.cove.test.FakeNodeBroadcast
import org.bitcoinppl.cove.test.FakeNodeFundedOutput
import org.bitcoinppl.cove.test.QrScript
import org.bitcoinppl.cove.test.fullLaunchDevice
import org.bitcoinppl.cove.test.waitFor
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

/**
 * Fills the Send amount from a scanned payment URI and from Max with the wallet in sats or in
 * fiat, and reads the confirm screen in sats, against cove-fake-node, then checks what the screen
 * shows and what the app broadcast
 *
 * The fake node prices bitcoin at 100,000 per BTC, so 1,000 sats is 1.00 in fiat. Runs in the
 * uiTest flavour only, like the other full-launch tests, and needs the fake node up and forwarded,
 * which `just android-instrumented-ci` does
 */
@LargeTest
@RunWith(AndroidJUnit4::class)
class SendFlowAmountSourcesFullLaunchTest {
    private companion object {
        const val COIN_SATS = 100_000L
        const val FUNDED_SATS = 2 * COIN_SATS
        const val PAYMENT_URI = "bitcoin:$EXTERNAL_ADDRESS?amount=0.0004"
        const val PAYMENT_SATS = 40_000L
    }

    @get:Rule
    val stayAwake = AndroidDeviceStayAwakeRule()

    private lateinit var device: UiDevice
    private lateinit var send: SendDisplayRobot
    private lateinit var qr: QrScript

    @Before
    fun prepareDevice() {
        device = fullLaunchDevice()
        send = SendDisplayRobot(device)
        qr = QrScript()
        send.prepareDevice()
    }

    @After
    fun restoreDevice() {
        qr.remove()
        send.restoreDevice()
    }

    /** A payment URI's amount opens Send in the wallet's unit, sats here, and is the amount sent */
    @Test
    fun scannedPaymentAmountShowsInSatsAndIsSent() {
        send.launchFundedWallet(listOf(COIN_SATS, COIN_SATS))
        send.showBalanceAs(BalanceDisplay.SATS)

        openSendFromScannedPaymentUri()
        send.waitForUnit("sats")
        send.waitForAmountText("40,000", "the URI's 0.0004 BTC didn't show as 40,000 sats")
        send.waitForSecondary("40.00", "40,000 sats didn't price as 40.00 in fiat")

        send.wallet.clickNext()
        send.wallet.swipeToSend()

        assertEquals("the URI's 0.0004 BTC must send 40,000 sats", PAYMENT_SATS, send.singleBroadcast().sentToExternal())
    }

    /** A payment URI's amount opens Send in fiat when the wallet is in fiat, and is the amount sent */
    @Test
    fun scannedPaymentAmountShowsInFiatAndIsSent() {
        send.launchFundedWallet(listOf(COIN_SATS, COIN_SATS))
        send.showBalanceAs(BalanceDisplay.FIAT_OVER_BTC)

        openSendFromScannedPaymentUri()
        send.waitForFiatAmount("40.00", "the URI's 0.0004 BTC didn't show as 40.00 in fiat")
        send.assertNoUnitSelector("fiat entry has no bitcoin unit selector")
        send.waitForSecondary("0.0004", "the URI's amount didn't show as 0.0004 BTC under the fiat field")

        send.wallet.clickNext()
        send.wallet.swipeToSend()

        assertEquals("the URI's 0.0004 BTC must send 40,000 sats", PAYMENT_SATS, send.singleBroadcast().sentToExternal())
    }

    /**
     * Max in sats fills the spendable balance less the fee and sends it all to the address, a
     * drain with no change output
     */
    @Test
    fun maxInSatsDrainsTheWallet() {
        val funded = send.launchFundedWallet(listOf(COIN_SATS, COIN_SATS))
        send.showBalanceAs(BalanceDisplay.SATS)

        send.wallet.startSend()
        send.waitForUnit("sats")
        val maxText = send.tapMax()
        assertTrue("Max showed \"$maxText\", not a grouped sats amount", ',' in maxText && '.' !in maxText)
        val maxSats = bitcoinAmountSats(maxText, "sats")
        send.waitForUnit("sats")

        send.wallet.clickNext()
        val fee = confirmSats(send.confirmValue(FEE_TAG))
        assertEquals("the confirm screen must pay the Max amount and the fee", maxSats + fee, confirmSats(send.confirmValue(TOTAL_TAG)))
        send.wallet.swipeToSend()

        assertDrains(send.singleBroadcast(), funded, fee)
        assertEquals("Max must send the $maxSats sats it showed", maxSats, send.singleBroadcast().sentToExternal())
    }

    /**
     * Max in fiat fills the spendable balance less the fee in fiat and still sends it all to the
     * address, rather than the amount the fiat text rounds to
     */
    @Test
    fun maxInFiatDrainsTheWallet() {
        val funded = send.launchFundedWallet(listOf(COIN_SATS, COIN_SATS))
        send.showBalanceAs(BalanceDisplay.FIAT_OVER_SATS)

        send.wallet.startSend()
        send.assertNoUnitSelector("fiat entry has no bitcoin unit selector")
        val maxText = send.tapMax()
        assertTrue("Max showed \"$maxText\", not a fiat amount to the cent", fiatDigits(maxText).substringAfter('.', "").length == 2)

        send.wallet.clickNext()
        val fee = confirmSats(send.confirmValue(FEE_TAG))
        val paid = confirmSats(send.confirmValue(TOTAL_TAG))
        send.wallet.swipeToSend()

        val broadcast = send.singleBroadcast()
        assertDrains(broadcast, funded, fee)
        assertEquals("the confirm screen's total must be the coins spent", FUNDED_SATS, paid)
        assertTrue(
            "Max showed \"$maxText\" in fiat, which isn't the ${broadcast.sentToExternal()} sats sent",
            fiatNear(maxText, broadcast.sentToExternal()),
        )
    }

    /** With the wallet in sats, the confirm screen's fee and totals read in sats and add up */
    @Test
    fun confirmScreenShowsFeeAndTotalInSats() {
        val funded = send.launchFundedWallet(listOf(COIN_SATS, COIN_SATS))
        send.showBalanceAs(BalanceDisplay.SATS)

        send.wallet.startSend()
        send.waitForUnit("sats")
        send.typeAmount("50000")
        send.waitForAmountText("50,000", "50000 typed in sats didn't show grouped")
        send.wallet.clickNext()

        val feeText = send.confirmValue(FEE_TAG)
        val payText = send.confirmValue(TOTAL_TAG)
        listOf(feeText, payText).forEach { text ->
            assertTrue("\"$text\" on the confirm screen isn't in sats", text.trim().endsWith(" SATS"))
        }

        val fee = confirmSats(feeText)
        assertTrue("a fee of $fee sats isn't a plausible fee", fee in 1 until 10_000)
        assertEquals("you will pay must be the amount and the fee", 50_000L + fee, confirmSats(payText))

        send.wallet.swipeToSend()

        val broadcast = send.singleBroadcast()
        assertEquals("50,000 typed in sats must send 50,000 sats", 50_000L, broadcast.sentToExternal())
        assertEquals("the broadcast must pay the fee the confirm screen showed", fee, broadcast.fee(funded))
    }

    // the wallet screen's scanner reads the URI, and its alert opens Send with the address and amount
    private fun openSendFromScannedPaymentUri() {
        send.scanFromWalletScreen(qr, PAYMENT_URI)
        device.waitFor(By.text("Send To Address"), timeoutMs = 20_000L).click()
        device.waitFor(By.res(SendDisplayRobot.AMOUNT_TAG), timeoutMs = 20_000L)
    }

    private fun assertDrains(
        broadcast: FakeNodeBroadcast,
        funded: List<FakeNodeFundedOutput>,
        fee: Long,
    ) {
        assertEquals("Max must spend every coin", funded.map { it.outpoint }.toSet(), broadcast.inputs.toSet())
        assertEquals("Max has no change output", listOf(EXTERNAL_ADDRESS), broadcast.outputs.map { it.address })
        assertEquals("Max must send everything but the fee the confirm screen showed", FUNDED_SATS - fee, broadcast.sentToExternal())
    }
}
