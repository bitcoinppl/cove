package org.bitcoinppl.cove.flows.SendFlow

import android.system.Os
import android.view.accessibility.AccessibilityNodeInfo
import android.view.accessibility.AccessibilityNodeInfo.AccessibilityAction
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.filters.LargeTest
import androidx.test.platform.app.InstrumentationRegistry
import androidx.test.uiautomator.By
import androidx.test.uiautomator.BySelector
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
import org.bitcoinppl.cove.test.TAPSIGNER_CVC
import org.bitcoinppl.cove.test.TEST_MNEMONIC
import org.bitcoinppl.cove.test.findText
import org.bitcoinppl.cove.test.fullLaunchDevice
import org.bitcoinppl.cove.test.hierarchy
import org.bitcoinppl.cove.test.launchFullApp
import org.bitcoinppl.cove.test.waitFor
import org.bitcoinppl.cove_core.Database
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import java.util.regex.Pattern

/**
 * Sends from the funded test wallet to another of the user's wallets through "Send between
 * wallets", and checks the Your Wallets sheet, the picked wallet's card and its Change link,
 * Done on its address moving on to the amount, editing the address after a pick, the TO WALLET card on Confirm with its hardware wallet note,
 * and the address the broadcast pays
 *
 * There are two other wallets. One is a hardware wallet imported from a second mnemonic's
 * watch-only descriptors through the debug NFC seam. The descriptors carry the key's fingerprint,
 * so it imports as a hardware wallet rather than an xpub-only one, and the signer host derives the
 * account's first receive address, which is the one a wallet with no history hands out next. The
 * other is an emulated TAPSIGNER, which has no screen to check an address on. Runs in the uiTest
 * flavour; needs the fake node, the hardware signer host and its TAPSIGNER bridge up and
 * forwarded, which `just android-instrumented-ci` does
 */
@LargeTest
@RunWith(AndroidJUnit4::class)
class SendToMyWalletFullLaunchTest {
    private companion object {
        const val FUNDED_SATS = 100_000L

        // a BIP39 test vector, so the destination has a fingerprint of its own; a key from the test
        // mnemonic would be refused as a duplicate of the sending wallet
        val DESTINATION_MNEMONIC = List(11) { "zoo" }.plus("wrong").joinToString(" ")

        const val TAPSIGNER_BRIDGE_ENV = "COVE_UI_TEST_TAPSIGNER_BRIDGE"
        const val CHECK_ON_DEVICE_NOTE = "Check this address on your hardware wallet before you send."
        const val KEYBOARD_REQUEST_SETTLE_MS = 1_000L
        val DESTINATION_ROW: Pattern = Pattern.compile("sendFlow\\.transferDestination\\..+")
        val SHEET_TITLE: BySelector = By.res("sendFlow.transferDestinations.title")
        val RECIPIENT_NAME: BySelector = By.res("sendFlow.transferRecipient.name")
        val CONFIRM_RECIPIENT_NAME: BySelector = By.res("sendFlowConfirm.transferRecipient.name")
        val FINGERPRINT: Regex = Regex("""\[([0-9a-fA-F]{8})/""")
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

        // the bridge answers every NFC scan, so it must not outlive this test in the process
        Os.unsetenv(TAPSIGNER_BRIDGE_ENV)

        when (val previous = previousShowImeWithHardKeyboard) {
            null, "null" -> device.executeShellCommand("settings delete secure show_ime_with_hard_keyboard")
            else -> device.executeShellCommand("settings put secure show_ime_with_hard_keyboard $previous")
        }
    }

    @Test
    fun pickingAnotherWalletFillsItsNextAddressAndSendsThere() {
        signer.requireHealthy()
        node.reset()
        signer.startTapSigner()

        val funded = node.fundWallet(TEST_MNEMONIC, listOf(0 to FUNDED_SATS)).single()
        node.mine()
        val (external, internal) = signer.watchOnlyDescriptors(DESTINATION_MNEMONIC)
        val fingerprint =
            requireNotNull(FINGERPRINT.find(external)?.groupValues?.get(1)?.uppercase()) { "the descriptor names no fingerprint" }

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

        // 2. the hardware wallet comes from Add Wallet's NFC scan; the sheet offers a wallet only once
        // its first scan has finished, which waitForBalance waits for
        screens.scanNfcFromAddWallet()
        nfc.showTextRecord("$external\n$internal")
        device.waitFor(By.text("Wallet imported successfully"), timeoutMs = 30_000L)
        device.waitFor(By.text("OK")).click()
        wallet.waitForBalance(0L)

        // 3. the TAPSIGNER comes next; the seams read the bridge at each scan, and it answers every
        // scan once set, so it waits until the hardware wallet is in
        HardwareSignerControl.useTapSignerBridge()
        screens.scanTapSignerFromAddWallet("Import TAPSIGNER?")
        screens.enterTapSignerCvc(TAPSIGNER_CVC)
        device.waitFor(By.text("Import Complete"), timeoutMs = 30_000L)
        device.waitFor(By.text("Continue")).click()
        wallet.waitForBalance(0L)

        // 4. Send from the funded wallet shows the address card and the row that opens the others
        selectWalletFromSidebar(sendingWallet)
        wallet.waitForBalance(FUNDED_SATS)
        device.waitFor(By.text("Send")).click()
        device.waitFor(By.text("Send to"), timeoutMs = 15_000L)
        device.waitFor(By.text("Paste or scan address"))

        // 5. the sheet lists both other wallets, the hardware wallet by its fingerprint, without the
        // sending wallet or any note
        openYourWallets("sendFlow.sendBetweenWallets")
        val rows = device.findObjects(By.res(DESTINATION_ROW))
        assertEquals("the sheet should list only the two other wallets", 2, rows.size)
        assertTrue(
            "the hardware wallet's row should show its fingerprint $fingerprint\n${device.hierarchy()}",
            device.hasObject(By.res(DESTINATION_ROW).hasDescendant(By.text(fingerprint))),
        )
        assertFalse("the sheet should carry no hardware wallet note", device.hasObject(By.textContains("hardware wallet")))

        // 6. picking the TAPSIGNER shows its card with its name and address
        pickTapSigner(fingerprint)
        val tapSignerName = device.waitFor(RECIPIENT_NAME, timeoutMs = 30_000L).text
        val pickedAddress = device.waitFor(By.res("sendFlow.transferRecipient.address")).text.unspaced()
        assertTrue("the card should show the picked address, it shows $pickedAddress", pickedAddress.startsWith("tb1"))

        // 7. Done on the address with no amount yet moves on to the amount, and the card comes back
        device.waitFor(By.res("sendFlow.transferRecipient.address")).click()
        wallet.waitUntil("tapping the card should go back to the address field") {
            device.findObject(By.res("sendFlow.address"))?.isFocused == true
        }
        pressImeDone("sendFlow.address")
        wallet.waitUntil("Done on the address should move to the amount field") { amountIsFocused() }
        waitForText(RECIPIENT_NAME, tapSignerName, "Done without an edit should keep the picked wallet")

        // 8. tapping the address goes back to editing it, and an edit drops the pick for good
        device.waitFor(By.res("sendFlow.transferRecipient.address")).click()
        wallet.waitUntil("tapping the card should go back to the address field") {
            device.findObject(By.res("sendFlow.address"))?.isFocused == true
        }
        device.executeShellCommand("input text x")
        wallet.waitUntil("the edit should reach the field") {
            device.findObject(By.res("sendFlow.address"))?.text?.contains("x") == true
        }
        device.waitFor(By.res("sendFlow.amount")).click()
        wallet.waitUntil("the address field should lose the focus") {
            device.findObject(By.res("sendFlow.address"))?.isFocused != true
        }
        assertFalse("an edited address should drop the picked wallet", device.wait(Until.hasObject(RECIPIENT_NAME), 3_000L))

        // 9. Confirm names the picked TAPSIGNER, with no note, since it has no screen
        openYourWallets("sendFlow.sendBetweenWallets")
        pickTapSigner(fingerprint)
        waitForText(RECIPIENT_NAME, tapSignerName, "picking the TAPSIGNER again should show its card")
        wallet.tapMax()
        wallet.clickNext()
        assertConfirmRecipient(tapSignerName)
        assertFalse("a TAPSIGNER has no screen to check the address on", device.hasObject(By.text(CHECK_ON_DEVICE_NOTE)))

        // 10. back on Set Amount the pick stays, and Change swaps it for the hardware wallet, whose next
        // address it fills in
        device.pressBack()
        waitForText(RECIPIENT_NAME, tapSignerName, "going back should keep the picked wallet")
        openYourWallets("sendFlow.transferRecipient.change")
        device.waitFor(By.res(DESTINATION_ROW).hasDescendant(By.text(fingerprint))).click()
        assertTrue("picking a wallet should close the sheet", device.wait(Until.gone(SHEET_TITLE), 10_000L))
        wallet.waitUntil("Change should swap the picked wallet") {
            device.findText(RECIPIENT_NAME).let { it != null && it != tapSignerName }
        }
        val coldName = device.findText(RECIPIENT_NAME).orEmpty()
        wallet.waitUntil("the hardware wallet's card should show ${destination.firstAddress}", timeoutMs = 30_000L) {
            showsAddress(destination.firstAddress)
        }

        // 11. Confirm names the hardware wallet with the check-on-device note, and the broadcast pays
        // its address
        wallet.clickNext()
        assertConfirmRecipient(coldName)
        device.waitFor(By.text(CHECK_ON_DEVICE_NOTE))
        wallet.waitUntil("Confirm should show ${destination.firstAddress}") {
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

    /**
     * Presses the keyboard's Done on the field tagged [tag] through its accessibility action: the test
     * keeps the soft keyboard hidden, and a hardware Enter types a new line in a multi-line field
     */
    private fun pressImeDone(tag: String) {
        val root = InstrumentationRegistry.getInstrumentation().uiAutomation.rootInActiveWindow
        val field = requireNotNull(root?.findByViewId(tag)) { "no $tag field\n${device.hierarchy()}" }
        assertTrue("the $tag field should take the Done action", field.performAction(AccessibilityAction.ACTION_IME_ENTER.id))
    }

    /** The amount's tag sits on its auto-size wrapper, so the focus lands on the text field inside it */
    private fun amountIsFocused(): Boolean =
        try {
            device.findObject(By.res("sendFlow.amount"))?.let { it.isFocused || it.hasObject(By.focused(true)) } == true
        } catch (_: StaleObjectException) {
            false
        }

    private fun AccessibilityNodeInfo.findByViewId(id: String): AccessibilityNodeInfo? {
        if (viewIdResourceName == id) return this

        return (0 until childCount).firstNotNullOfOrNull { getChild(it)?.findByViewId(id) }
    }

    private fun openYourWallets(opener: String) {
        revealSendTo(By.res(opener))
        device.waitFor(By.res(opener)).click()
        assertEquals("Your Wallets", device.waitFor(SHEET_TITLE).text)
    }

    /**
     * Brings [target] in the Send to section on screen: the amount field takes the focus when Send
     * opens and after a pick, and its keyboard covers the section
     */
    private fun revealSendTo(target: BySelector) {
        val deadline = System.currentTimeMillis() + 20_000L

        while (System.currentTimeMillis() < deadline) {
            // Set Amount asks for the keyboard 350 ms after a focus change, which waitForIdle
            // doesn't wait out, and a covered row stays in the hierarchy, so it closes the keyboard
            // again after that before trusting the row
            wallet.dismissKeyboard()
            Thread.sleep(KEYBOARD_REQUEST_SETTLE_MS)
            wallet.dismissKeyboard()
            if (device.wait(Until.hasObject(target), 1_000L)) return

            device.swipe(device.displayWidth / 2, device.displayHeight * 2 / 3, device.displayWidth / 2, device.displayHeight / 3, 20)
            device.waitForIdle()
        }

        error("$target never showed in the Send to section\n${device.hierarchy()}")
    }

    /** Picks the row that doesn't show the hardware wallet's fingerprint */
    private fun pickTapSigner(fingerprint: String) {
        val tapSigner =
            device.findObjects(By.res(DESTINATION_ROW)).single { row ->
                !row.hasObject(By.text(fingerprint))
            }

        tapSigner.click()
        assertTrue("picking a wallet should close the sheet", device.wait(Until.gone(SHEET_TITLE), 10_000L))
    }

    private fun assertConfirmRecipient(name: String) {
        device.waitFor(By.text("TO WALLET"), timeoutMs = 30_000L)
        waitForText(CONFIRM_RECIPIENT_NAME, name, "Confirm should name the picked wallet")
    }

    private fun waitForText(
        selector: BySelector,
        text: String,
        message: String,
    ) {
        wallet.waitUntil("$message: $text") { device.findText(selector) == text }
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
