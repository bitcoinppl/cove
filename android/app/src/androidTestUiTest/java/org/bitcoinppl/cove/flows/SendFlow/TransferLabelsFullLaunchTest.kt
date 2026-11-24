package org.bitcoinppl.cove.flows.SendFlow

import android.system.Os
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.filters.LargeTest
import androidx.test.uiautomator.By
import androidx.test.uiautomator.UiDevice
import org.bitcoinppl.cove.test.AndroidDeviceStayAwakeRule
import org.bitcoinppl.cove.test.FAKE_NODE_ESPLORA_URL
import org.bitcoinppl.cove.test.FakeNodeControl
import org.bitcoinppl.cove.test.FullLaunchOnboardingRobot
import org.bitcoinppl.cove.test.FullLaunchStartupRobot
import org.bitcoinppl.cove.test.FullLaunchWalletRobot
import org.bitcoinppl.cove.test.HardwareWalletScreens
import org.bitcoinppl.cove.test.QrScript
import org.bitcoinppl.cove.test.SECOND_MNEMONIC
import org.bitcoinppl.cove.test.TEST_MNEMONIC
import org.bitcoinppl.cove.test.fullLaunchDevice
import org.bitcoinppl.cove.test.hierarchy
import org.bitcoinppl.cove.test.launchFullApp
import org.junit.After
import org.junit.Assert.assertNotEquals
import org.junit.Before
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

/**
 * Sends between two of the user's wallets in the full app against cove-fake-node and checks that
 * each wallet's transaction list labels the transfer with the other wallet's name
 *
 * The first wallet is the funded test mnemonic and the second the empty legal winner phrase,
 * scanned in through the debug QR seam. Runs in the uiTest flavour only, like the other full-launch
 * tests, and needs the fake node up and forwarded, which `just android-instrumented-ci` does
 */
@LargeTest
@RunWith(AndroidJUnit4::class)
class TransferLabelsFullLaunchTest {
    private companion object {
        const val FUNDED_SATS = 100_000L

        /** The second mnemonic's first receive address */
        const val OTHER_RECEIVE_0 = "tb1qjgx204hxfwuse548jc34fjzg6ffq8pvrqwlex4"
    }

    @get:Rule
    val stayAwake = AndroidDeviceStayAwakeRule()

    private lateinit var device: UiDevice
    private lateinit var qr: QrScript
    private val node = FakeNodeControl()
    private val wallet by lazy { FullLaunchWalletRobot(device) }
    private var previousShowImeWithHardKeyboard: String? = null

    @Before
    fun prepareDevice() {
        device = fullLaunchDevice()
        qr = QrScript()

        // see UtxoLockFullLaunchTest: the onboarding robot must never see the soft keyboard, and
        // sends price fees from the fake node rather than mainnet mempool.space
        previousShowImeWithHardKeyboard = device.executeShellCommand("settings get secure show_ime_with_hard_keyboard").trim()
        device.executeShellCommand("settings put secure show_ime_with_hard_keyboard 0")
        Os.setenv("COVE_FEE_URL", "$FAKE_NODE_ESPLORA_URL/api/v1/fees/recommended", true)
    }

    @After
    fun restoreDevice() {
        qr.remove()
        when (val previous = previousShowImeWithHardKeyboard) {
            null, "null" -> device.executeShellCommand("settings delete secure show_ime_with_hard_keyboard")
            else -> device.executeShellCommand("settings put secure show_ime_with_hard_keyboard $previous")
        }
    }

    /**
     * A Max send from the first wallet to the second shows as "Transfer to" the second in the first
     * wallet and, once mined, as "Transfer from" the first in the second
     */
    @Test
    fun sendBetweenWalletsIsLabelledInBoth() {
        node.reset()
        node.fundWallet(TEST_MNEMONIC, listOf(0 to FUNDED_SATS))
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
        wallet.waitForBalance(FUNDED_SATS)
        val sender = requireNotNull(wallet.selectedWalletName()) { "the first wallet has no name\n${device.hierarchy()}" }

        wallet.importFromAddWallet(SECOND_MNEMONIC, qr, sats = 0L)
        val receiver = requireNotNull(wallet.selectedWalletName()) { "the second wallet has no name\n${device.hierarchy()}" }
        assertNotEquals("both wallets have the same name, so the labels can't tell them apart", sender, receiver)

        wallet.selectWallet(sender)
        wallet.waitForBalance(FUNDED_SATS)
        wallet.startSend(OTHER_RECEIVE_0)
        wallet.tapMax()
        wallet.clickNext()
        wallet.swipeToSend()

        val send = node.broadcasts().single()
        val received =
            requireNotNull(send.outputs.firstOrNull { it.address == OTHER_RECEIVE_0 }) {
                "the send didn't pay the second wallet: $send"
            }
        assertTransactionRow("Transfer to $receiver")

        node.mine()
        wallet.selectWallet(receiver)
        HardwareWalletScreens(device, wallet).refreshUntilBalance(received.sats)
        assertTransactionRow("Transfer from $sender")
    }

    private fun assertTransactionRow(label: String) {
        wallet.waitUntil("expected the transaction list to show $label", timeoutMs = 30_000L) {
            device.hasObject(By.text(label))
        }
    }
}
