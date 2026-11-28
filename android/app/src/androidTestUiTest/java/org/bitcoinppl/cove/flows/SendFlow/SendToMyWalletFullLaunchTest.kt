package org.bitcoinppl.cove.flows.SendFlow

import android.system.Os
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.filters.LargeTest
import androidx.test.uiautomator.By
import androidx.test.uiautomator.StaleObjectException
import androidx.test.uiautomator.UiDevice
import androidx.test.uiautomator.Until
import org.bitcoinppl.cove.test.AndroidDeviceStayAwakeRule
import org.bitcoinppl.cove.test.FAKE_NODE_ESPLORA_URL
import org.bitcoinppl.cove.test.FakeNodeControl
import org.bitcoinppl.cove.test.FullLaunchOnboardingRobot
import org.bitcoinppl.cove.test.FullLaunchStartupRobot
import org.bitcoinppl.cove.test.FullLaunchWalletRobot
import org.bitcoinppl.cove.test.HardwareSignerControl
import org.bitcoinppl.cove.test.HardwareWalletScreens
import org.bitcoinppl.cove.test.NfcNdefScript
import org.bitcoinppl.cove.test.PublicKeyAddressType
import org.bitcoinppl.cove.test.TEST_MNEMONIC
import org.bitcoinppl.cove.test.fullLaunchDevice
import org.bitcoinppl.cove.test.launchFullApp
import org.bitcoinppl.cove.test.waitFor
import org.bitcoinppl.cove_core.Database
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import java.util.regex.Pattern

/**
 * Sends from the funded test wallet to another of the user's wallets through the send screen's
 * wallet picker, and checks the picker's list, the address a pick fills in, the recipient Confirm
 * shows and the address the broadcast pays
 *
 * The other wallet is a hardware wallet imported from a second mnemonic's watch-only descriptors
 * through the debug NFC seam. The descriptors carry the key's fingerprint, so it imports as a
 * hardware wallet rather than an xpub-only one, and the signer host derives the account's first
 * receive address, which is the one a wallet with no history hands out next. Runs in the uiTest flavour; needs the fake node and the
 * hardware signer host up and forwarded, which `just android-instrumented-ci` does
 */
@LargeTest
@RunWith(AndroidJUnit4::class)
class SendToMyWalletFullLaunchTest {
    private companion object {
        const val FUNDED_SATS = 100_000L

        // a BIP39 test vector, so the destination has a fingerprint of its own; a key from the test
        // mnemonic would be refused as a duplicate of the sending wallet
        val DESTINATION_MNEMONIC = List(11) { "zoo" }.plus("wrong").joinToString(" ")

        const val PICKER_TITLE = "Send to My Wallet"
        val DESTINATION_ROW: Pattern = Pattern.compile("sendFlow\\.transferDestination\\..+")
    }

    @get:Rule
    val stayAwake = AndroidDeviceStayAwakeRule()

    private lateinit var device: UiDevice
    private lateinit var wallet: FullLaunchWalletRobot
    private lateinit var screens: HardwareWalletScreens
    private lateinit var nfc: NfcNdefScript
    private val node = FakeNodeControl()
    private val signer = HardwareSignerControl()
    private var previousShowImeWithHardKeyboard: String? = null

    @Before
    fun prepareDevice() {
        device = fullLaunchDevice()
        wallet = FullLaunchWalletRobot(device)
        screens = HardwareWalletScreens(device, wallet)
        nfc = NfcNdefScript()

        // see UtxoLockFullLaunchTest: the onboarding robot must never see the soft keyboard
        previousShowImeWithHardKeyboard = device.executeShellCommand("settings get secure show_ime_with_hard_keyboard").trim()
        device.executeShellCommand("settings put secure show_ime_with_hard_keyboard 0")

        // debug builds read this on every fee fetch, so sends price fees from the fake node
        Os.setenv("COVE_FEE_URL", "$FAKE_NODE_ESPLORA_URL/api/v1/fees/recommended", true)
    }

    @After
    fun restoreDevice() {
        nfc.remove()

        when (val previous = previousShowImeWithHardKeyboard) {
            null, "null" -> device.executeShellCommand("settings delete secure show_ime_with_hard_keyboard")
            else -> device.executeShellCommand("settings put secure show_ime_with_hard_keyboard $previous")
        }
    }

    @Test
    fun pickingAnotherWalletFillsItsNextAddressAndSendsThere() {
        signer.requireHealthy()
        node.reset()

        val funded = node.fundWallet(TEST_MNEMONIC, listOf(0 to FUNDED_SATS)).single()
        node.mine()
        val (external, internal) = signer.watchOnlyDescriptors(DESTINATION_MNEMONIC)

        // the same BIP84 account as the descriptors, for the address the host derives at /0/0
        val destination = signer.publicKeyExport(PublicKeyAddressType.NATIVE_SEGWIT, DESTINATION_MNEMONIC)

        // 1. a fresh install on the fake node imports the funded wallet from onboarding
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
        val sendingWallet = Database().wallets().all().single().name

        // 2. the destination comes from Add Wallet's NFC scan; the picker offers a wallet only once
        // its first scan has finished, which waitForBalance waits for
        screens.scanNfcFromAddWallet()
        nfc.showTextRecord("$external\n$internal")
        device.waitFor(By.text("Wallet imported successfully"), timeoutMs = 30_000L)
        device.waitFor(By.text("OK")).click()
        wallet.waitForBalance(0L)

        // 3. back on the funded wallet, the picker lists the other wallet as a hardware wallet, and
        // not the sending wallet
        selectWalletFromSidebar(sendingWallet)
        wallet.waitForBalance(FUNDED_SATS)
        device.waitFor(By.text("Send")).click()
        device.waitFor(By.res("sendFlow.sendToMyWallet"), timeoutMs = 15_000L).click()
        device.waitFor(By.text(PICKER_TITLE))

        val rows = device.findObjects(By.res(DESTINATION_ROW))
        assertEquals("the picker should list only the other wallet", 1, rows.size)
        device.waitFor(By.text("Hardware wallet"))
        device.waitFor(By.textContains("Check the address on the device"))

        rows.single().click()
        assertTrue("picking a wallet should close the picker", device.wait(Until.gone(By.text(PICKER_TITLE)), 10_000L))

        // 4. the pick fills in the destination's next receive address; the unfocused field shows it
        // as grouped text over an empty editor
        wallet.waitUntil("the pick should fill in ${destination.firstAddress}", timeoutMs = 30_000L) {
            showsAddress(destination.firstAddress)
        }

        // 5. Confirm names that address as the recipient, and the broadcast pays it
        wallet.tapMax()
        wallet.clickNext()
        wallet.waitUntil("Confirm should name ${destination.firstAddress} as the recipient") {
            showsAddress(destination.firstAddress)
        }
        wallet.swipeToSend()

        val sent = node.broadcasts().single()
        assertEquals(listOf(funded.outpoint), sent.inputs)
        assertTrue(
            "the send should pay ${destination.firstAddress}, it paid ${sent.outputs}",
            sent.outputs.any { it.address == destination.firstAddress },
        )
    }

    private fun selectWalletFromSidebar(name: String) {
        wallet.waitUntil("expected Menu to open the sidebar") {
            try {
                device.findObject(By.desc("Menu"))?.click()
            } catch (_: StaleObjectException) {
            }

            device.wait(Until.hasObject(By.text("Add Wallet")), 2_000L)
        }

        device.waitFor(By.text(name)).click()
    }

    // the app spaces the address out in groups
    private fun showsAddress(address: String): Boolean =
        try {
            device.findObjects(By.textStartsWith(address.take(4))).any { it.text?.unspaced() == address }
        } catch (_: StaleObjectException) {
            false
        }

    private fun String.unspaced(): String = filterNot { it.isWhitespace() }
}
