package org.bitcoinppl.cove.flows.SendFlow

import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.filters.LargeTest
import org.bitcoinppl.cove.test.AndroidDeviceStayAwakeRule
import org.bitcoinppl.cove.test.fullLaunchDevice
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Before
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

/**
 * Sends in the unit picked on the wallet screen against cove-fake-node, then checks what the app
 * broadcast
 *
 * Mirrors iOS's `SendFlowUnitFullLaunchUITests`. Runs in the uiTest flavour only, like the other
 * full-launch tests. Needs the fake node up and forwarded, which `just android-instrumented-ci` does
 */
@LargeTest
@RunWith(AndroidJUnit4::class)
class SendFlowUnitFullLaunchTest {
    private companion object {
        const val COIN_SATS = 100_000L
    }

    @get:Rule
    val stayAwake = AndroidDeviceStayAwakeRule()

    private lateinit var send: SendDisplayRobot

    @Before
    fun prepareDevice() {
        send = SendDisplayRobot(fullLaunchDevice())
        send.prepareDevice()
    }

    @After
    fun restoreDevice() {
        send.restoreDevice()
    }

    @Test
    fun satsPickedOnWalletScreenIsTheSendAmountUnit() {
        send.launchFundedWallet(listOf(COIN_SATS, COIN_SATS))

        // 1: show the balance in sats, then send an amount typed in sats
        send.showBalanceAs(BalanceDisplay.SATS)
        send.wallet.startSend()
        send.waitForUnit("sats")
        send.typeAmount("50000")
        send.wallet.clickNext()
        send.wallet.swipeToSend()

        val satsSends = send.node.broadcasts()
        assertEquals("expected exactly one broadcast after the sats send", 1, satsSends.size)
        assertEquals("50000 typed in sats must send 50,000 sats", 50_000L, satsSends[0].sentToExternal())

        // 2: a second Send in the same launch opens in sats, and switching it to btc converts what
        // was typed and sends in btc
        send.wallet.startSend()
        send.waitForUnit("sats")
        send.typeAmount("20000")
        send.selectUnit("btc")
        send.waitForBitcoinAmount("0.0002", "20000 sats didn't convert to 0.0002 btc")
        send.wallet.clickNext()
        send.wallet.swipeToSend()

        val btcSends = send.node.broadcasts()
        assertEquals("expected the sats send and the btc send", 2, btcSends.size)
        assertEquals("0.0002 btc must send 20,000 sats", 20_000L, btcSends[1].sentToExternal())
    }
}
