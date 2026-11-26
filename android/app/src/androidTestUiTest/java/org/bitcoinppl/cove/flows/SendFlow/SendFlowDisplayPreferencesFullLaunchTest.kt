package org.bitcoinppl.cove.flows.SendFlow

import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.filters.LargeTest
import androidx.test.uiautomator.By
import androidx.test.uiautomator.UiDevice
import org.bitcoinppl.cove.flows.SendFlow.SendDisplayRobot.Companion.MASKED_AMOUNT
import org.bitcoinppl.cove.test.AndroidDeviceStayAwakeRule
import org.bitcoinppl.cove.test.fullLaunchDevice
import org.bitcoinppl.cove.test.waitFor
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

/**
 * Changes the send flow's display preferences on the Send and coin control screens against
 * cove-fake-node, then checks what the screen shows and what the app broadcast
 *
 * Mirrors iOS's `SendFlowDisplayPreferencesFullLaunchUITests`. The fake node prices bitcoin at
 * 100,000 per BTC, so 0.0005 BTC is 50.00 in fiat. Runs in the uiTest flavour only, like the other
 * full-launch tests, and needs the fake node up and forwarded, which `just android-instrumented-ci`
 * does
 */
@LargeTest
@RunWith(AndroidJUnit4::class)
class SendFlowDisplayPreferencesFullLaunchTest {
    private companion object {
        const val COIN_SATS = 100_000L

        // big enough that 50,000 sats with a digit typed after it is still spendable
        const val LARGE_COIN_SATS = 400_000L
    }

    @get:Rule
    val stayAwake = AndroidDeviceStayAwakeRule()

    private lateinit var device: UiDevice
    private lateinit var send: SendDisplayRobot
    private var previousShowImeWithHardKeyboard: String? = null

    @Before
    fun prepareDevice() {
        device = fullLaunchDevice()
        send = SendDisplayRobot(device)

        // see UtxoLockFullLaunchTest: the onboarding robot must never see the soft keyboard
        previousShowImeWithHardKeyboard = device.executeShellCommand("settings get secure show_ime_with_hard_keyboard").trim()
        device.executeShellCommand("settings put secure show_ime_with_hard_keyboard 0")

        send.useFakeNodeFeesAndPrices()
    }

    @After
    fun restoreDevice() {
        when (val previous = previousShowImeWithHardKeyboard) {
            null, "null" -> device.executeShellCommand("settings delete secure show_ime_with_hard_keyboard")
            else -> device.executeShellCommand("settings put secure show_ime_with_hard_keyboard $previous")
        }
    }

    /**
     * Switching the Send screen between bitcoin and fiat entry converts what was typed, and the
     * amount typed in fiat is what gets sent
     */
    @Test
    fun fiatEntryOnTheSendScreenSendsTheTypedAmount() {
        send.launchFundedWallet(listOf(COIN_SATS, COIN_SATS))

        send.wallet.startSend()
        send.waitForUnit("btc")
        send.typeAmount("0.0005")
        send.waitForSecondary("50.00", "0.0005 BTC never showed as 50.00 in fiat")

        // to fiat: the field shows the fiat amount and the row under it the bitcoin amount
        send.tapSecondary()
        send.waitForFiatAmount("50.00", "switching to fiat didn't show 50.00 in the amount field")
        send.assertNoUnitSelector("fiat entry has no bitcoin unit selector")
        send.waitForSecondary("0.0005", "the bitcoin amount didn't move under the fiat field")

        send.replaceAmount("30") { fiatDigits(send.amountText()) in setOf("30", "30.00") }
        send.waitForFiatAmount("30", "the fiat field didn't take 30")
        send.waitForSecondary("0.0003", "30.00 in fiat didn't price as 0.0003 BTC")

        // back to bitcoin: the field shows what 30.00 bought, in btc
        send.tapSecondary()
        send.waitForUnit("btc")
        send.waitForBitcoinAmount("0.0003", "switching back to btc didn't convert 30.00")

        send.wallet.clickNext()
        send.wallet.swipeToSend()

        assertEquals("30.00 typed in fiat must send 30,000 sats", 30_000L, send.singleBroadcast().sentToExternal())
    }

    /** The Send header's eye hides the fiat amount under the amount field, and shows it again */
    @Test
    fun hidingAmountsFromTheSendHeaderMasksTheFiatAmount() {
        send.launchFundedWallet(listOf(COIN_SATS, COIN_SATS))

        send.wallet.startSend()
        send.waitForUnit("btc")
        send.typeAmount("0.0005")
        send.waitForSecondary("50.00", "0.0005 BTC never showed as 50.00 in fiat")

        send.tapHeaderVisibility()
        send.waitForSecondary(MASKED_AMOUNT, "hiding amounts left the fiat amount showing")
        assertFalse("the masked row still shows 50.00", "50.00" in send.secondaryText())
        send.waitForBitcoinAmount("0.0005", "hiding amounts changed the typed amount")

        send.tapHeaderVisibility()
        send.waitForSecondary("50.00", "showing amounts again didn't bring back the fiat amount")
        assertFalse("the fiat row is still masked", MASKED_AMOUNT in send.secondaryText())
    }

    /**
     * Switching the Send screen from btc to sats converts what was typed, and digits typed after
     * that are read as sats
     */
    @Test
    fun switchingTheSendScreenToSatsConvertsAndParsesInSats() {
        send.launchFundedWallet(listOf(LARGE_COIN_SATS, LARGE_COIN_SATS))

        send.wallet.startSend()
        send.waitForUnit("btc")
        send.typeAmount("0.0005")

        send.selectUnit("sats")
        send.waitForAmountText("50,000", "0.0005 BTC didn't convert to 50,000 sats")
        send.waitForSecondary("50.00", "50,000 sats didn't price as 50.00 in fiat")

        // read in btc, 50,0005 isn't a number the field accepts; in sats it is 500,005
        send.appendToAmount("5")
        send.waitForAmountText("500,005", "a digit typed after 50,000 sats wasn't read as sats")
        send.waitForUnit("sats")
        send.waitForSecondary("500.0", "500,005 sats didn't price as about 500.00 in fiat")

        send.wallet.clickNext()
        send.wallet.swipeToSend()

        assertEquals("500,005 typed in sats must send 500,005 sats", 500_005L, send.singleBroadcast().sentToExternal())
    }

    /**
     * Coin control's amount screen shows the amount in the unit picked there, with the fiat amount
     * under it, and sends the amount it shows
     */
    @Test
    fun coinControlSendsTheAmountShownInTheUnitPickedThere() {
        send.launchFundedWallet(listOf(COIN_SATS, COIN_SATS))

        send.openManageUtxos()
        send.selectAllCoins()
        send.continueToCoinControlAmount()
        send.wallet.enterAddress()

        // both coins less the fee, so the amount itself is read back rather than assumed
        send.selectUnit("btc")
        val btcText = send.waitForCoinControlAmount("both coins less the fee never showed in btc")
        assertTrue("\"$btcText\" isn't an amount in btc", '.' in btcText)
        val sats = bitcoinAmountSats(btcText, "btc")
        assertTrue("coin control showed $sats sats, not both coins less a fee", sats in (2 * COIN_SATS - 10_000L) until 2 * COIN_SATS)
        send.waitForCoinControlFiatNear(sats, "$sats sats never showed in fiat")

        send.selectUnit("sats")
        val satsText = send.waitForCoinControlAmount("switching coin control to sats didn't show the amount")
        assertEquals("switching coin control to sats didn't convert $btcText btc", sats.toString(), satsText)
        assertTrue("$satsText sats has no grouping commas", ',' in send.coinControlAmountText().orEmpty())
        send.waitForCoinControlFiatNear(sats, "switching to sats changed the fiat amount")

        send.selectUnit("btc")
        val backToBtc = send.waitForCoinControlAmount("switching coin control back to btc didn't show the amount")
        assertEquals("switching coin control back to btc didn't convert $satsText sats", btcText, backToBtc)

        send.wallet.clickNext()
        send.wallet.swipeToSend()

        val broadcast = send.singleBroadcast()
        assertEquals("coin control must send the $sats sats it showed", sats, broadcast.sentToExternal())
        assertEquals("spending both coins whole leaves no change", 1, broadcast.outputs.size)
    }

    /** An amount typed in sats into coin control's custom amount sheet is the amount sent */
    @Test
    fun coinControlCustomAmountTypedInSatsIsSent() {
        send.launchFundedWallet(listOf(COIN_SATS, COIN_SATS))

        send.openManageUtxos()
        send.selectAllCoins()
        send.continueToCoinControlAmount()
        send.wallet.enterAddress()
        send.selectUnit("sats")
        send.waitForCoinControlAmount("both coins less the fee never showed in sats")

        send.openCustomAmountSheet()
        device.waitFor(By.text("SATS"))
        send.wallet.typeInto(By.res(SendDisplayRobot.CUSTOM_AMOUNT_TAG), "150000") {
            send.customAmountText()?.replace(",", "") == "150000"
        }
        send.closeCustomAmountSheet()

        assertEquals("the custom amount didn't reach the amount screen", "150000", send.waitForCoinControlAmount("the custom amount never showed"))
        send.waitForCoinControlFiatNear(150_000L, "150,000 sats didn't price as 150.00 in fiat")

        send.wallet.clickNext()
        send.wallet.swipeToSend()

        assertEquals("the custom 150,000 sats must be sent", 150_000L, send.singleBroadcast().sentToExternal())
    }
}
