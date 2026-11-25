package org.bitcoinppl.cove.flows.SendFlow

import android.system.Os
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.filters.LargeTest
import androidx.test.uiautomator.By
import androidx.test.uiautomator.UiDevice
import org.bitcoinppl.cove.test.AndroidDeviceStayAwakeRule
import org.bitcoinppl.cove.test.EXTERNAL_ADDRESS
import org.bitcoinppl.cove.test.FAKE_NODE_ESPLORA_URL
import org.bitcoinppl.cove.test.FakeNodeBroadcast
import org.bitcoinppl.cove.test.FakeNodeControl
import org.bitcoinppl.cove.test.FullLaunchOnboardingRobot
import org.bitcoinppl.cove.test.FullLaunchStartupRobot
import org.bitcoinppl.cove.test.FullLaunchWalletRobot
import org.bitcoinppl.cove.test.TEST_MNEMONIC
import org.bitcoinppl.cove.test.findText
import org.bitcoinppl.cove.test.fullLaunchDevice
import org.bitcoinppl.cove.test.launchFullApp
import org.bitcoinppl.cove.test.waitFor
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
        const val FUNDED_SATS = 100_000L
        const val UNIT_TOGGLE_TAG = "selectedWallet.balanceUnitToggle"
        const val BALANCE_TAG = "selectedWallet.balance"
        const val AMOUNT_TAG = "sendFlow.amount"
        const val UNIT_TAG = "sendFlow.unit"

        // the balance cycles through four btc, fiat and sats combinations, so this many taps reach any
        const val UNIT_TOGGLE_STATES = 4
    }

    @get:Rule
    val stayAwake = AndroidDeviceStayAwakeRule()

    private lateinit var device: UiDevice
    private lateinit var wallet: FullLaunchWalletRobot
    private val node = FakeNodeControl()
    private var previousShowImeWithHardKeyboard: String? = null

    @Before
    fun prepareDevice() {
        device = fullLaunchDevice()
        wallet = FullLaunchWalletRobot(device)

        // see UtxoLockFullLaunchTest: the onboarding robot must never see the soft keyboard
        previousShowImeWithHardKeyboard = device.executeShellCommand("settings get secure show_ime_with_hard_keyboard").trim()
        device.executeShellCommand("settings put secure show_ime_with_hard_keyboard 0")

        // debug builds read this on every fee fetch, so sends price fees from the fake node
        Os.setenv("COVE_FEE_URL", "$FAKE_NODE_ESPLORA_URL/api/v1/fees/recommended", true)
    }

    @After
    fun restoreDevice() {
        when (val previous = previousShowImeWithHardKeyboard) {
            null, "null" -> device.executeShellCommand("settings delete secure show_ime_with_hard_keyboard")
            else -> device.executeShellCommand("settings put secure show_ime_with_hard_keyboard $previous")
        }
    }

    @Test
    fun satsPickedOnWalletScreenIsTheSendAmountUnit() {
        node.reset()
        node.fundWallet(TEST_MNEMONIC, listOf(0 to FUNDED_SATS, 1 to FUNDED_SATS))
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
        wallet.waitForBalance(2 * FUNDED_SATS)

        // 1: show the balance in sats, then send an amount typed in sats
        useSatsOnWalletScreen()
        wallet.startSend()
        waitForUnit("sats")
        typeAmount("50000")
        wallet.clickNext()
        wallet.swipeToSend()

        val satsSends = node.broadcasts()
        assertEquals("expected exactly one broadcast after the sats send", 1, satsSends.size)
        assertEquals("50000 typed in sats must send 50,000 sats", 50_000L, sentToExternal(satsSends[0]))

        // 2: switching the send screen back to btc converts what was typed and sends in btc
        wallet.startSend()
        waitForUnit("sats")
        typeAmount("20000")
        selectUnit("btc")
        waitForAmountText("0.0002", "20000 sats didn't convert to 0.0002 btc")
        wallet.clickNext()
        wallet.swipeToSend()

        val btcSends = node.broadcasts()
        assertEquals("expected the sats send and the btc send", 2, btcSends.size)
        assertEquals("0.0002 btc must send 20,000 sats", 20_000L, sentToExternal(btcSends[1]))
    }

    // MARK: wallet screen

    // a tap on the balance steps through btc, fiat over btc, sats and fiat over sats, so tap until
    // the balance itself reads in sats, which also leaves the send screen out of fiat mode
    private fun useSatsOnWalletScreen() {
        repeat(UNIT_TOGGLE_STATES) {
            val before = device.findText(By.res(BALANCE_TAG))
            if (before.showsSats()) return

            device.waitFor(By.res(UNIT_TOGGLE_TAG)).click()
            wallet.waitUntil("the balance didn't change after tapping it, still \"$before\"", timeoutMs = 5_000L) {
                device.findText(By.res(BALANCE_TAG)) != before
            }
        }

        wallet.waitUntil("the balance never showed sats", timeoutMs = 10_000L) {
            device.findText(By.res(BALANCE_TAG)).showsSats()
        }
    }

    private fun String?.showsSats(): Boolean = this?.trim()?.endsWith(" SATS", ignoreCase = true) == true

    // MARK: send screen

    private fun waitForUnit(unit: String) {
        wallet.waitUntil("expected the send amount unit to read $unit", timeoutMs = 10_000L) {
            device.findText(By.res(UNIT_TAG)) == unit
        }
    }

    private fun typeAmount(amount: String) {
        wallet.typeInto(By.res(AMOUNT_TAG), amount) { amountText() == amount }
    }

    // sats get grouping commas as they're typed
    private fun amountText(): String? = device.findText(By.res(AMOUNT_TAG))?.replace(",", "")

    // waitUntil appends the hierarchy, which shows what the field held
    private fun waitForAmountText(
        text: String,
        message: String,
    ) {
        wallet.waitUntil(message, timeoutMs = 10_000L) { amountText() == text }
    }

    private fun selectUnit(unit: String) {
        device.waitFor(By.res(UNIT_TAG)).click()
        device.waitFor(By.res("$UNIT_TAG.$unit"), timeoutMs = 5_000L).click()
        waitForUnit(unit)
    }

    private fun sentToExternal(broadcast: FakeNodeBroadcast): Long = broadcast.outputs.single { it.address == EXTERNAL_ADDRESS }.sats
}
