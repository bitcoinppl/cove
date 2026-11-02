package org.bitcoinppl.cove.flows.SendFlow

import android.content.ClipData
import android.content.ClipboardManager
import android.content.Context
import android.os.Build
import android.system.Os
import android.util.Base64
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.filters.LargeTest
import androidx.test.platform.app.InstrumentationRegistry
import androidx.test.uiautomator.By
import androidx.test.uiautomator.UiDevice
import androidx.test.uiautomator.Until
import org.bitcoinppl.cove.test.AndroidDeviceStayAwakeRule
import org.bitcoinppl.cove.test.EXTERNAL_ADDRESS
import org.bitcoinppl.cove.test.FAKE_NODE_ESPLORA_URL
import org.bitcoinppl.cove.test.FakeNodeControl
import org.bitcoinppl.cove.test.FakeNodeFundedOutput
import org.bitcoinppl.cove.test.FullLaunchOnboardingRobot
import org.bitcoinppl.cove.test.FullLaunchStartupRobot
import org.bitcoinppl.cove.test.FullLaunchWalletRobot
import org.bitcoinppl.cove.test.HardwareSignerControl
import org.bitcoinppl.cove.test.QrScript
import org.bitcoinppl.cove.test.SignedFrames
import org.bitcoinppl.cove.test.TAPSIGNER_CVC
import org.bitcoinppl.cove.test.TEST_MNEMONIC
import org.bitcoinppl.cove.test.findText
import org.bitcoinppl.cove.test.fullLaunchDevice
import org.bitcoinppl.cove.test.hierarchy
import org.bitcoinppl.cove.test.launchFullApp
import org.bitcoinppl.cove.test.parseSats
import org.bitcoinppl.cove.test.scrollTo
import org.bitcoinppl.cove.test.waitFor
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Assume.assumeFalse
import org.junit.Before
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import java.math.BigDecimal
import java.util.regex.Pattern

/**
 * Signs sends from hardware wallets in the real app: animated UR and BBQr signatures over the QR
 * seam, signed PSBTs that match no draft, and TAPSIGNER signing over the NFC seam
 *
 * Debug builds read QR frames from the file `COVE_UI_TEST_QR_SCRIPT` names instead of the camera,
 * and send TAPSIGNER commands to the card emulator at `COVE_UI_TEST_TAPSIGNER_BRIDGE` instead of an
 * NFC tag; docs/testing.md describes both seams. The test sets both in process before launching.
 *
 * Runs in the uiTest flavour only, like the other full-launch tests. Needs the fake node and the
 * hardware signer host up and forwarded, which `just android-instrumented-ci` does
 */
@LargeTest
@RunWith(AndroidJUnit4::class)
class HardwareSigningFullLaunchTest {
    private companion object {
        val FUNDED_SATS = listOf(100_000L, 60_000L)

        // enough frames that the scanner shows progress before it completes
        const val ANIMATED_PARTS = 5
        const val WRONG_CVC = "000000"

        // what the TAPSIGNER sheet shows when signing fails for any reason but the CVC
        const val SIGNING_FAILED = "Connection failed, please try again"
    }

    @get:Rule
    val stayAwake = AndroidDeviceStayAwakeRule()

    private lateinit var device: UiDevice
    private lateinit var wallet: FullLaunchWalletRobot
    private lateinit var qr: QrScript
    private val node = FakeNodeControl()
    private val signer = HardwareSignerControl()
    private var previousShowImeWithHardKeyboard: String? = null

    @Before
    fun prepareDevice() {
        device = fullLaunchDevice()
        wallet = FullLaunchWalletRobot(device)
        qr = QrScript()

        // see UtxoLockFullLaunchTest: the onboarding robot must never see the soft keyboard
        previousShowImeWithHardKeyboard = device.executeShellCommand("settings get secure show_ime_with_hard_keyboard").trim()
        device.executeShellCommand("settings put secure show_ime_with_hard_keyboard 0")

        // debug builds read this on every fee fetch, so sends price fees from the fake node
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

    // MARK: QR signing

    @Test
    fun animatedUrSignatureReachesConfirmationAndBroadcasts() {
        signWithAnimatedQrAndBroadcast(SignedFrames::ur, "UR")
    }

    @Test
    fun animatedBbqrSignatureReachesConfirmationAndBroadcasts() {
        signWithAnimatedQrAndBroadcast(SignedFrames::bbqr, "BBQr")
    }

    private fun signWithAnimatedQrAndBroadcast(
        frames: (SignedFrames) -> List<String>,
        formatLabel: String,
    ) {
        val funded = fundTestWallet()
        importWatchOnlyWallet()
        startMaxDraft()

        val signed = signer.sign(exportUnsignedPsbt(formatLabel), ANIMATED_PARTS)
        val animated = frames(signed)
        assertTrue("the signature must be an animated QR", animated.size > 1)

        // the camera can catch an animated QR at any frame, so feed them out of order
        openSignatureScanner()
        qr.show(animated.reversed())

        assertConfirmationScreen()
        assertAdvancedDetails()
        wallet.swipeToSend()

        val broadcast = node.broadcasts().single()
        assertEquals("a Max send spends every UTXO", funded.map { it.outpoint }.toSet(), broadcast.inputs.toSet())
        assertEquals(listOf(EXTERNAL_ADDRESS), broadcast.outputs.map { it.address })
    }

    @Test
    fun partialBbqrFramesShowProgressUntilTheRestArrive() {
        fundTestWallet()
        importWatchOnlyWallet()
        startMaxDraft()

        val frames = signer.sign(exportUnsignedPsbt("BBQr"), ANIMATED_PARTS).bbqr
        val total = frames.size
        assertTrue(total >= 3)

        // two frames in, the scanner shows how far it got and stays open
        openSignatureScanner()
        qr.show(frames.take(2))
        waitForScanProgress("Scanned 2 of $total")
        waitForScanProgress("${total - 2} parts left")
        assertFalse(device.hasObject(By.res("sendFlowConfirm.swipeToSend")))

        // a repeated frame doesn't count twice
        qr.show(listOf(frames[1]))
        qr.waitUntilScanned(frameCount = 1)
        waitForScanProgress("Scanned 2 of $total")

        qr.show(frames.drop(2))
        assertConfirmationScreen()
    }

    // MARK: QR error paths

    @Test
    fun signaturesThatMatchNoDraftAreRejected() {
        fundTestWallet()
        importWatchOnlyWallet()

        // sign a draft, then delete it
        startMaxDraft()
        val deleted = signer.sign(exportUnsignedPsbt("UR"), 1)
        deleteDraft()

        // a different draft, so the deleted one's signature can't match it
        startDraftOfHalfTheBalance()

        openSignatureScanner()
        qr.show(deleted.ur)
        assertNoUnsignedTransactionAlert()

        openSignatureScanner()
        qr.show(signer.unrelatedSignedPsbt("signet", 3).ur)
        assertNoUnsignedTransactionAlert()

        // nothing checks a PSBT's network, so a mainnet one is just another unknown transaction
        openSignatureScanner()
        qr.show(signer.unrelatedSignedPsbt("bitcoin", 3).bbqr)
        assertNoUnsignedTransactionAlert()

        // the paste path names the missing transaction too
        setClipboard(hex(deleted.psbt))
        device.waitFor(By.text("Import Signature")).click()
        device.waitFor(By.text("Paste")).click()
        device.waitFor(By.text("Paste Error"))
        device.waitFor(By.text("Transaction not found in pending transactions."))
        device.waitFor(By.text("OK")).click()

        device.waitFor(By.text("Import Signature"))
    }

    // MARK: TAPSIGNER

    @Test
    fun tapSignerImportRetriesAfterWrongPinAndDroppedSession() {
        node.reset()
        signer.startTapSigner()
        HardwareSignerControl.useTapSignerBridge()
        launchSignetApp()
        importTestMnemonicToFinishOnboarding()

        startTapSignerImport()
        enterCvc(WRONG_CVC)
        tryAgainAfterWrongPin()

        // the card leaves the field partway through the import
        signer.dropNextTapSignerSession(afterExchanges = 1)
        enterCvc(TAPSIGNER_CVC)
        retryAfterDroppedSession()

        finishTapSignerImport()
    }

    @Test
    fun tapSignerSignsAfterWrongPinAndDroppedSession() {
        node.reset()
        signer.startTapSigner()
        HardwareSignerControl.useTapSignerBridge()
        launchSignetApp()
        importTestMnemonicToFinishOnboarding()

        startTapSignerImport()
        enterCvc(TAPSIGNER_CVC)
        finishTapSignerImport()

        val funded = node.fund(receiveAddress(), listOf(100_000L))
        refreshUntilBalance(100_000L)

        wallet.startSend()
        wallet.tapMax()
        wallet.clickNext(expecting = By.text("Sign using TAPSIGNER"))

        device.waitFor(By.text("Sign using TAPSIGNER")).click()
        enterCvc(WRONG_CVC)

        // rust-cktap rejects every BIP84 PSBT before it contacts the card, so even the wrong CVC
        // ends in the generic failure instead of Wrong PIN. Only that outcome skips the rest; once
        // signing reaches the card, the whole flow runs
        val outcome =
            checkNotNull(device.wait(Until.findObject(By.text(Pattern.compile("Wrong PIN|$SIGNING_FAILED"))), 30_000L)) {
                "the wrong CVC showed neither Wrong PIN nor a signing failure"
            }
        assumeFalse(
            "TAPSIGNER signing fails with Invalid path, see https://github.com/bitcoinppl/cove/issues/965",
            outcome.text == SIGNING_FAILED,
        )
        tryAgainAfterWrongPin()

        signer.dropNextTapSignerSession(afterExchanges = 1)
        enterCvc(TAPSIGNER_CVC)
        retryAfterDroppedSession()

        assertConfirmationScreen()
        wallet.swipeToSend()

        val broadcast = node.broadcasts().single()
        assertEquals(funded.map { it.outpoint }, broadcast.inputs)
        assertEquals(listOf(EXTERNAL_ADDRESS), broadcast.outputs.map { it.address })
    }

    /**
     * Runs steps a known bug breaks. It passes while they fail and fails once they pass, so the test
     * is updated when the bug is fixed, like XCTExpectFailure on iOS
     */
    // MARK: fixture

    /** Funds external indexes 0 and 1 of the test mnemonic, each its own UTXO */
    private fun fundTestWallet(): List<FakeNodeFundedOutput> {
        signer.requireHealthy()
        node.reset()

        val outputs = node.fundWallet(TEST_MNEMONIC, FUNDED_SATS.mapIndexed { index, sats -> index to sats })
        node.mine()

        assertEquals(FUNDED_SATS, outputs.map { it.sats })
        return outputs
    }

    private fun launchSignetApp() {
        launchFullApp()
        FullLaunchStartupRobot(device).assertBootstrappedIntoOnboarding()
        wallet.selectSignetFakeNode()
    }

    /** Imports the test mnemonic's account as a watch-only hardware wallet by scanning its export */
    private fun importWatchOnlyWallet() {
        launchSignetApp()
        FullLaunchOnboardingRobot(device)
            .tapGetStarted()
            .chooseExistingUser()
            .useAnotherWallet()
            .chooseHardwareWallet()
            .openHardwareQrScanner()

        val (external, internal) = signer.watchOnlyDescriptors()
        qr.show(listOf("$external\n$internal"))

        device.waitFor(By.text("Protect this hardware wallet with Cloud Backup?"), timeoutMs = 30_000L)
        FullLaunchOnboardingRobot(device)
            .skipCloudBackupAfterImport()
            .acceptTermsAfterImport()

        wallet.waitForBalance(FUNDED_SATS.sum())
    }

    /** TAPSIGNER import starts from Add Wallet, which needs onboarding finished first */
    private fun importTestMnemonicToFinishOnboarding() {
        FullLaunchOnboardingRobot(device)
            .tapGetStarted()
            .chooseExistingUser()
            .useAnotherWallet()
            .chooseSoftwareWallet()
            .importKnownEmptyMainnetWalletWords()
            .skipCloudBackupAfterImport()
            .acceptTermsAfterImport()

        device.waitFor(By.res("selectedWallet.balance"), timeoutMs = 30_000L)
    }

    // MARK: hardware screen

    private fun startMaxDraft() {
        wallet.startSend()
        wallet.tapMax()
        wallet.clickNext(expecting = By.text("Import Signature"))
    }

    private fun startDraftOfHalfTheBalance() {
        wallet.startSend()
        wallet.enterAmount(BigDecimal(FUNDED_SATS.sum() / 2).movePointLeft(8).toPlainString())
        wallet.clickNext(expecting = By.text("Import Signature"))
    }

    /** Shows the draft as an animated QR in the format, then copies the unsigned PSBT */
    private fun exportUnsignedPsbt(formatLabel: String): String {
        device.waitFor(By.text("Export Transaction")).click()
        device.waitFor(By.text("QR Code")).click()
        device.waitFor(By.text("Scan this QR"), timeoutMs = 15_000L)

        // a small PSBT can fit in one BBQr frame, which shows no frame counter
        device.waitFor(By.text(formatLabel)).click()
        device.waitFor(By.desc("QR code for transaction signing"))

        device.waitFor(By.desc("Copy")).click()
        val psbt = clipboardText().trim()
        assertTrue("the Copy button copied nothing", psbt.isNotEmpty())

        device.pressBack()
        device.waitFor(By.text("Import Signature"))

        return psbt
    }

    private fun openSignatureScanner() {
        device.waitFor(By.text("Import Signature")).click()
        device.waitFor(By.text("QR")).click()
        device.waitFor(By.text("Scan QR Code"))
    }

    private fun deleteDraft() {
        device.waitFor(By.desc("Delete")).click()
        device.wait(Until.gone(By.text("Import Signature")), 10_000L)

        // deleting returns to the send screen, which Back leaves for the wallet
        wallet.dismissKeyboard()
        repeat(3) {
            if (device.wait(Until.hasObject(By.res("selectedWallet.balance")), 3_000L)) return
            device.pressBack()
        }

        device.waitFor(By.res("selectedWallet.balance"))
    }

    private fun assertNoUnsignedTransactionAlert() {
        device.waitFor(By.text("No Unsigned Transaction Found"), timeoutMs = 15_000L)
        device.waitFor(By.textStartsWith("No unsigned transaction found for transaction"))
        device.waitFor(By.text("OK")).click()
        device.waitFor(By.text("Import Signature"))
    }

    /**
     * Waits for the scanner's progress text, reading the screen afresh on each try
     *
     * The progress text is the last thing that changes on an otherwise idle screen, and
     * uiautomator answers from the accessibility node cache, so a content change the cache misses
     * stays hidden from it for good
     */
    private fun waitForScanProgress(text: String) {
        val automation = InstrumentationRegistry.getInstrumentation().uiAutomation
        val deadline = System.currentTimeMillis() + 20_000L

        while (System.currentTimeMillis() < deadline) {
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.UPSIDE_DOWN_CAKE) automation.clearCache()
            if (device.hasObject(By.text(text))) return

            Thread.sleep(250)
        }

        error("Timed out waiting for scan progress \"$text\"\n${device.hierarchy()}")
    }

    private fun assertConfirmationScreen() {
        device.waitFor(By.res("sendFlowConfirm.swipeToSend"), timeoutMs = 30_000L)
    }

    private fun assertAdvancedDetails() {
        device.scrollTo(By.text("Address")).click()

        device.waitFor(By.text("Advanced Details"))
        device.waitFor(By.text("UTXOs Used"), timeoutMs = 15_000L)
        device.waitFor(By.text("Sent To Address"))
        device.waitFor(By.text("Fee"))

        // a Max send has no change output
        assertFalse(device.hasObject(By.text("UTXO Change")))

        device.waitFor(By.desc("Close")).click()
        assertTrue(device.wait(Until.gone(By.text("Advanced Details")), 5_000L))
    }

    // MARK: TAPSIGNER screens

    /** Adds a wallet from an NFC tag read, which the seam answers from the emulated card */
    private fun startTapSignerImport() {
        device.waitFor(By.desc("Menu")).click()
        device.waitFor(By.text("Add Wallet")).click()
        device.waitFor(By.desc("More wallet options")).click()
        device.waitFor(By.text("NFC")).click()

        device.waitFor(By.text("Import TAPSIGNER?"), timeoutMs = 15_000L)
        device.waitFor(By.text("Yes")).click()
    }

    // the TAPSIGNER sheet is its own window, which doesn't expose test tags as resource ids
    private fun enterCvc(cvc: String) {
        device.waitFor(By.text("Enter TAPSIGNER CVC"), timeoutMs = 15_000L)
        // the field masks its text, so check the digit count beneath it
        wallet.typeInto(By.clazz("android.widget.EditText"), cvc) {
            device.hasObject(By.text("Digits: ${cvc.length}"))
        }
        wallet.dismissKeyboard()
        device.waitFor(By.text("Submit").enabled(true)).click()
    }

    private fun tryAgainAfterWrongPin() {
        device.waitFor(By.text("Wrong PIN"), timeoutMs = 30_000L)
        device.waitFor(By.text("Try Again")).click()
    }

    /**
     * Retries once the dropped session's error shows: an operation the card had already started
     * offers Retry, which resumes it, and one it hadn't keeps the PIN sheet open for the CVC again
     */
    private fun retryAfterDroppedSession() {
        val retry = By.text("Continue Import")
        val failed = By.textEndsWith("Please try again")
        val connectionFailed = By.text("Connection failed, please try again")

        wallet.waitUntil("expected the dropped session to show an error", timeoutMs = 45_000L) {
            device.hasObject(retry) || device.hasObject(failed) || device.hasObject(connectionFailed)
        }

        if (device.hasObject(retry)) {
            device.findObject(retry).click()
        } else {
            enterCvc(TAPSIGNER_CVC)
        }
    }

    private fun finishTapSignerImport() {
        device.waitFor(By.text("Import Complete"), timeoutMs = 30_000L)
        device.waitFor(By.text("Continue")).click()
        device.waitFor(By.res("selectedWallet.balance"), timeoutMs = 30_000L)
    }

    /** The wallet only looks for new transactions on its own now and then, so pull to refresh */
    private fun refreshUntilBalance(sats: Long) {
        wallet.waitUntil("expected a balance of $sats sats after refreshing", timeoutMs = 90_000L) {
            if (device.findText(By.res("selectedWallet.balance"))?.let(::parseSats) == sats) return@waitUntil true

            device.swipe(device.displayWidth / 2, device.displayHeight * 2 / 5, device.displayWidth / 2, device.displayHeight * 9 / 10, 30)
            Thread.sleep(3_000)
            false
        }

        wallet.waitForBalance(sats)
    }

    private fun receiveAddress(): String {
        device.waitFor(By.text("Receive")).click()

        // Copy Address does nothing until the address loads, and closes the sheet once it copies
        wallet.waitUntil("expected Copy Address to copy the receive address") {
            device.findObject(By.text("Copy Address"))?.click()
            device.wait(Until.gone(By.text("Copy Address")), 2_000L)
        }

        return clipboardText().trim()
    }

    // MARK: clipboard

    private fun clipboardText(): String {
        var text = ""
        val instrumentation = InstrumentationRegistry.getInstrumentation()
        instrumentation.runOnMainSync {
            val clipboard = instrumentation.targetContext.getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager
            text = clipboard.primaryClip?.getItemAt(0)?.text?.toString().orEmpty()
        }

        return text
    }

    private fun setClipboard(text: String) {
        val instrumentation = InstrumentationRegistry.getInstrumentation()
        instrumentation.runOnMainSync {
            val clipboard = instrumentation.targetContext.getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager
            clipboard.setPrimaryClip(ClipData.newPlainText("signed PSBT", text))
        }
    }

    private fun hex(base64: String): String = Base64.decode(base64, Base64.DEFAULT).joinToString("") { "%02x".format(it) }
}
