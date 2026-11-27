package org.bitcoinppl.cove.flows.SendFlow

import android.system.Os
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.filters.LargeTest
import androidx.test.uiautomator.By
import androidx.test.uiautomator.UiDevice
import androidx.test.uiautomator.Until
import org.bitcoinppl.cove.test.AndroidDeviceStayAwakeRule
import org.bitcoinppl.cove.test.EXTERNAL_ADDRESS
import org.bitcoinppl.cove.test.FAKE_NODE_ESPLORA_URL
import org.bitcoinppl.cove.test.FakeNodeControl
import org.bitcoinppl.cove.test.FullLaunchOnboardingRobot
import org.bitcoinppl.cove.test.FullLaunchStartupRobot
import org.bitcoinppl.cove.test.FullLaunchWalletRobot
import org.bitcoinppl.cove.test.QrScript
import org.bitcoinppl.cove.test.SECOND_MNEMONIC
import org.bitcoinppl.cove.test.TEST_MNEMONIC
import org.bitcoinppl.cove.test.findText
import org.bitcoinppl.cove.test.fullLaunchDevice
import org.bitcoinppl.cove.test.hierarchy
import org.bitcoinppl.cove.test.launchFullApp
import org.bitcoinppl.cove.test.scrollTo
import org.bitcoinppl.cove.test.waitFor
import org.junit.After
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotEquals
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

/**
 * Checks the note under the send address field that names which of the user's wallets owns the
 * address, on Set Amount and on coin control's Set Amount, in the full app against cove-fake-node
 *
 * The first wallet is the funded test mnemonic and the second the empty legal winner phrase,
 * scanned in through the debug QR seam. Runs in the uiTest flavour only, like the other full-launch
 * tests, and needs the fake node up and forwarded, which `just android-instrumented-ci` does
 */
@LargeTest
@RunWith(AndroidJUnit4::class)
class AddressOwnershipSendFullLaunchTest {
    private companion object {
        const val SATS_A = 100_000L
        const val SATS_B = 60_000L

        /** The test mnemonic's receive address at index 2, which no send reveals */
        const val SENDER_RECEIVE_2 = "tb1qxdyjf6h5d6qxap4n2dap97q4j5ps6ua8sll0ct"

        /** The second mnemonic's first receive address */
        const val OTHER_RECEIVE_0 = "tb1qjgx204hxfwuse548jc34fjzg6ffq8pvrqwlex4"

        /** The second mnemonic's first change address */
        const val OTHER_CHANGE_0 = "tb1qmapywu8v8uztsqk4j8f5sarjzm2j9r04n8lauv"

        const val NOTE_TAG = "sendFlow.addressOwnership"
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

        // see UtxoLockFullLaunchTest: the onboarding robot must never see the soft keyboard
        previousShowImeWithHardKeyboard = device.executeShellCommand("settings get secure show_ime_with_hard_keyboard").trim()
        device.executeShellCommand("settings put secure show_ime_with_hard_keyboard 0")

        // fee rates come from the fake node, so Set Amount doesn't depend on mempool.space
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
     * The note names the sending wallet for its own address and the other wallet by name for one
     * of its addresses, follows the address as it is edited, and stays away for an address no
     * wallet owns
     */
    @Test
    fun setAmountNamesTheWalletThatOwnsTheAddress() {
        node.reset()
        node.fundWallet(TEST_MNEMONIC, listOf(0 to SATS_A, 1 to SATS_B))
        node.mine()

        val (sender, other) = importBothWallets()

        // Set Amount, from the sender's own address to the other wallet's, then to a stranger's
        wallet.startSend(SENDER_RECEIVE_2)
        assertOwnershipNote("This address belongs to this wallet, “$sender”")

        wallet.enterAddress(OTHER_RECEIVE_0)
        assertOwnershipNote("This address belongs to your wallet “$other”")

        wallet.enterAddress(EXTERNAL_ADDRESS)
        assertNoOwnershipNote()

        // coin control's own Set Amount screen
        wallet.dismissKeyboard()
        device.waitFor(By.res("sendFlow.back")).click()
        wallet.waitUntil("expected Back to return to $sender") { wallet.selectedWalletName() == sender }

        openManageUtxos()
        device.waitFor(By.res("coinControl.selectAll").text("Select All")).click()
        device.waitFor(By.res("coinControl.continue")).click()
        wallet.enterAddress(OTHER_CHANGE_0)
        assertOwnershipNote("This address belongs to your wallet “$other”")
    }

    /**
     * With no amount yet, a whole address moves the focus to the amount, and the note has to stay
     * in sight above the keyboard while the amount is typed
     */
    @Test
    fun noteStaysInSightWhileTheAmountIsTyped() {
        node.reset()
        node.fundWallet(TEST_MNEMONIC, listOf(0 to SATS_A))
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
        wallet.waitForBalance(SATS_A)
        val sender = requireNotNull(wallet.selectedWalletName()) { "the wallet has no name\n${device.hierarchy()}" }

        // onboarding is done, so the soft keyboard can show beside the emulator's hardware one, as
        // it does on a phone, which is the keyboard the note must stay above
        device.executeShellCommand("settings put secure show_ime_with_hard_keyboard 1")

        device.waitFor(By.text("Send")).click()
        device.waitFor(By.res("sendFlow.amount"))
        device.scrollTo(By.res("sendFlow.address")).click()
        device.waitFor(By.res("sendFlow.address").focused(true))
        device.findObject(By.res("sendFlow.address")).text = SENDER_RECEIVE_2
        device.waitFor(By.res("sendFlow.amount").hasDescendant(By.focused(true)))

        // a node the keyboard covers is left out of the accessibility tree, so a covered note never
        // shows up here
        assertOwnershipNote("This address belongs to this wallet, “$sender”")
        device.waitForIdle()
        val note = device.findObject(By.res(NOTE_TAG)).visibleBounds
        val toolbar = device.findObject(By.res("sendFlow.keyboardToolbar"))?.visibleBounds
        assertTrue("the keyboard closed, so this didn't check the note beside it", toolbar != null)
        assertTrue("the keyboard toolbar covers the note: it lies at $note, the toolbar at $toolbar", note.bottom <= toolbar!!.top)
    }

    /**
     * Imports the funded test mnemonic, then the empty second wallet, and returns to the first;
     * returns both wallets' names
     */
    private fun importBothWallets(): Pair<String, String> {
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
        wallet.waitForBalance(SATS_A + SATS_B)
        val sender = requireNotNull(wallet.selectedWalletName()) { "the first wallet has no name\n${device.hierarchy()}" }

        wallet.importFromAddWallet(SECOND_MNEMONIC, qr, sats = 0L)
        val other = requireNotNull(wallet.selectedWalletName()) { "the second wallet has no name\n${device.hierarchy()}" }
        assertNotEquals("both wallets have the same name, so the note can't tell them apart", sender, other)

        wallet.selectWallet(sender)
        wallet.waitForBalance(SATS_A + SATS_B)
        return sender to other
    }

    private fun openManageUtxos() {
        device.waitFor(By.desc("More")).click()
        device.waitFor(By.text("More Options"))

        // the sheet animates open half expanded, which can move Manage UTXOs under a tap or
        // leave it below the fold
        repeat(3) {
            device.waitForIdle()
            device.findObject(By.res("selectedWallet.manageUtxos"))?.click()
                ?: device.scrollTo(By.res("selectedWallet.manageUtxos")).click()
            if (device.wait(Until.hasObject(By.res("coinControl.selectAll")), 5_000L)) return
        }

        device.waitFor(By.res("coinControl.selectAll"))
    }

    private fun assertOwnershipNote(expected: String) {
        wallet.waitUntil("expected the note \"$expected\", showing \"${device.findText(By.res(NOTE_TAG))}\"") {
            device.findText(By.res(NOTE_TAG)) == expected
        }
    }

    // the lookup for an address no wallet owns ends quietly, so give it time to finish before
    // checking nothing showed
    private fun assertNoOwnershipNote() {
        wallet.waitUntil("expected no note for an address no wallet owns") { !device.hasObject(By.res(NOTE_TAG)) }
        Thread.sleep(3_000L)
        assertFalse(
            "a note showed for an address no wallet owns: ${device.findText(By.res(NOTE_TAG))}",
            device.hasObject(By.res(NOTE_TAG)),
        )
    }
}
