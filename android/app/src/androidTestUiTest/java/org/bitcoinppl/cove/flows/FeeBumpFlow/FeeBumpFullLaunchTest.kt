package org.bitcoinppl.cove.flows.FeeBumpFlow

import android.system.Os
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.filters.LargeTest
import androidx.test.platform.app.InstrumentationRegistry
import androidx.test.uiautomator.By
import androidx.test.uiautomator.BySelector
import androidx.test.uiautomator.UiDevice
import androidx.test.uiautomator.Until
import org.bitcoinppl.cove.AppManager
import org.bitcoinppl.cove.test.AndroidDeviceStayAwakeRule
import org.bitcoinppl.cove.test.EXTERNAL_ADDRESS
import org.bitcoinppl.cove.test.FAKE_NODE_ESPLORA_URL
import org.bitcoinppl.cove.test.FakeNodeBroadcast
import org.bitcoinppl.cove.test.FakeNodeControl
import org.bitcoinppl.cove.test.FullLaunchOnboardingRobot
import org.bitcoinppl.cove.test.FullLaunchStartupRobot
import org.bitcoinppl.cove.test.FullLaunchWalletRobot
import org.bitcoinppl.cove.test.HardwareSignerControl
import org.bitcoinppl.cove.test.HardwareWalletScreens
import org.bitcoinppl.cove.test.QrScript
import org.bitcoinppl.cove.test.TEST_MNEMONIC
import org.bitcoinppl.cove.test.anyText
import org.bitcoinppl.cove.test.findText
import org.bitcoinppl.cove.test.fullLaunchDevice
import org.bitcoinppl.cove.test.hierarchy
import org.bitcoinppl.cove.test.launchFullApp
import org.bitcoinppl.cove.test.scrollTo
import org.bitcoinppl.cove.test.waitFor
import org.bitcoinppl.cove.test.waitForText
import org.bitcoinppl.cove_core.AppAction
import org.bitcoinppl.cove_core.types.ColorSchemeSelection
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

/**
 * Speeds up pending sends in the real app against cove-fake-node, then checks the node holds the
 * replacement
 *
 * Runs in the uiTest flavour only, like the other full-launch tests. Needs the fake node and the
 * hardware signer host up and forwarded, which `just android-instrumented-ci` does. With the
 * instrumentation argument `coveScreenshotDir` set to a device path, such as
 * `/sdcard/Download/rbf-screens`, each new screen is saved there in light and dark
 */
@LargeTest
@RunWith(AndroidJUnit4::class)
class FeeBumpFullLaunchTest {
    private companion object {
        val FUNDED_SATS = listOf(100_000L, 60_000L)

        // what every test sends, small enough beside the funding that a bump never warns
        const val SEND_AMOUNT_BTC = "0.0005"

        // waiting after 20 seconds and delayed after 40, since a Slow send pays under Medium
        const val PENDING_SEND_DELAY_ENV = "COVE_UI_TEST_PENDING_SEND_DELAY_SECS"
        const val PENDING_SEND_DELAY_SECS = "40"

        const val RECEIVED_SATS = 25_000L
        const val CUSTOM_SLIDER_POSITION = 0.15
    }

    @get:Rule
    val stayAwake = AndroidDeviceStayAwakeRule()

    private lateinit var device: UiDevice
    private lateinit var wallet: FullLaunchWalletRobot
    private lateinit var screens: HardwareWalletScreens
    private lateinit var capture: ScreenCapture
    private lateinit var qr: QrScript
    private val node = FakeNodeControl()
    private val signer = HardwareSignerControl()
    private var previousShowImeWithHardKeyboard: String? = null

    @Before
    fun prepareDevice() {
        device = fullLaunchDevice()
        wallet = FullLaunchWalletRobot(device)
        screens = HardwareWalletScreens(device, wallet)
        capture = ScreenCapture(device)
        qr = QrScript()

        // see UtxoLockFullLaunchTest: the onboarding robot must never see the soft keyboard
        previousShowImeWithHardKeyboard = device.executeShellCommand("settings get secure show_ime_with_hard_keyboard").trim()
        device.executeShellCommand("settings put secure show_ime_with_hard_keyboard 0")

        // debug builds read this on every fee fetch, so sends and bumps price fees from the fake node
        Os.setenv("COVE_FEE_URL", "$FAKE_NODE_ESPLORA_URL/api/v1/fees/recommended", true)
    }

    @After
    fun restoreDevice() {
        qr.remove()
        Os.unsetenv(PENDING_SEND_DELAY_ENV)

        when (val previous = previousShowImeWithHardKeyboard) {
            null, "null" -> device.executeShellCommand("settings delete secure show_ime_with_hard_keyboard")
            else -> device.executeShellCommand("settings put secure show_ime_with_hard_keyboard $previous")
        }
    }

    // MARK: hot wallet

    @Test
    fun bumpFromDetailsReplacesTheSendAtMedium() {
        fundTestWallet()
        launchAndImportHotWallet()

        val original = sendSlowPayment()
        openDetails(original.txid)

        // a send that isn't late yet offers only the small button
        val speedUp = device.waitFor(By.res("transactionDetails.speedUp"), timeoutMs = 30_000L)
        assertFalse(device.hasObject(By.res("transactionDetails.delayedBanner")))
        capture("details-pending")

        speedUp.click()
        assertBumpSheetSelects("medium")
        assertNote("speedUp.note", contains = "The amount and recipient stay the same.")
        capture("bump-sheet")

        continueToConfirm()
        capture("confirm-bump")
        swipeAndFinishFeeBump()

        val replacement = assertReplaced(original)

        // the replacement's details strike through the fee it replaced
        openDetails(replacement.txid)
        expandDetails()
        device.scrollTo(By.res("transactionDetails.previousFee"))

        val networkFee = device.findText(By.res("transactionDetails.networkFee")).orEmpty()
        assertTrue("unexpected fee row '$networkFee'", networkFee.startsWith("New Network Fee"))

        device.scrollTo(By.res("transactionDetails.speedUpNote"))
        assertFalse("one bump has no fee history to show", device.hasObject(By.res("transactionDetails.feeHistory")))
        capture("details-bumped")
    }

    @Test
    fun maxSendBumpTakesTheFeeFromTheAmountSent() {
        fundTestWallet()
        launchAndImportHotWallet()
        wallet.waitForBalance(FUNDED_SATS.sum())

        wallet.startSend()
        wallet.tapMax()
        wallet.clickNext()
        wallet.swipeToSend()

        val original = node.broadcasts().single()
        assertEquals("a Max send has no change", 1, original.outputs.size)
        openBumpSheetFromMenu(original.txid)

        // with no change to pay from, the recipient gets less by the extra fee
        assertBumpSheetSelects("medium")
        assertNote("speedUp.note", contains = "The recipient receives ")
        assertNote("speedUp.note", contains = " sats less.")
        capture("bump-sheet-max-send")

        continueToConfirm()
        device.scrollTo(By.res("sendFlowConfirm.recipientReceivesLess"))
        capture("confirm-bump-max-send")
        swipeAndFinishFeeBump()

        val replacement = assertReplaced(original)
        assertEquals("the bump adds no change", 1, replacement.outputs.size)
        assertTrue(replacement.outputs[0].sats < original.outputs[0].sats)
    }

    @Test
    fun delayedPopupBumpsInOneTap() {
        // the list reads this on every rebuild, so setting it in process before launch is enough
        Os.setenv(PENDING_SEND_DELAY_ENV, PENDING_SEND_DELAY_SECS, true)

        fundTestWallet()
        launchAndImportHotWallet()

        val original = sendSlowPayment()

        // BySelector's builders change it in place, so each wait takes a fresh row selector
        fun rowWith(tag: String) = transactionRow(original.txid).hasDescendant(By.res(tag))

        device.waitFor(rowWith("transactionRow.pendingStatus"), timeoutMs = 60_000L)
        capture("home-waiting")

        device.waitFor(rowWith("transactionRow.delayedIcon"), timeoutMs = 60_000L)
        capture("home-delayed")

        // the delayed send's details lead with the banner
        openDetails(original.txid)
        device.waitFor(By.res("transactionDetails.delayedBanner"), timeoutMs = 30_000L)
        assertTrue(device.hasObject(By.res("transactionDetails.delayedBanner.speedUp")))
        assertFalse(device.hasObject(By.res("transactionDetails.speedUp")))
        capture("details-delayed")

        device.pressBack()
        device
            .waitFor(rowWith("transactionRow.delayedIcon"), timeoutMs = 15_000L)
            .findObject(By.res("transactionRow.delayedIcon"))
            .click()

        device.waitFor(By.res("speedUpPopup"), timeoutMs = 15_000L)
        assertTrue(device.hasObject(By.text("Taking longer than usual")))
        device.waitFor(By.res("speedUpPopup.suggestedFee"), timeoutMs = 30_000L)
        capture("delayed-popup")

        device.waitFor(By.res("speedUpPopup.speedUp").enabled(true)).click()
        assertConfirmInBumpMode()
        swipeAndFinishFeeBump()

        assertReplaced(original)
    }

    @Test
    fun customRateBumpThenSecondBumpShowsFeeHistory() {
        fundTestWallet()
        launchAndImportHotWallet()

        // the first bump picks its own rate from the long-press sheet
        val original = sendSlowPayment()
        openBumpSheetFromMenu(original.txid)

        device.waitFor(By.res("speedUp.option.medium"), timeoutMs = 30_000L)
        device.scrollTo(By.res("speedUp.option.custom")).click()
        device.waitFor(By.res("speedUp.customSheet"), timeoutMs = 10_000L)

        // a tap along the slider sets the rate there, a little above the lowest a bump takes
        val slider = device.waitFor(By.res("customFeeRate.slider")).visibleBounds
        device.click(slider.left + (slider.width() * CUSTOM_SLIDER_POSITION).toInt(), slider.centerY())
        wallet.waitUntil("the custom rate was never priced", timeoutMs = 15_000L) {
            device.findText(By.res("customFeeRate.total"))?.contains("sats") == true
        }
        capture("custom-rate")

        device.waitFor(By.res("customFeeRate.done").enabled(true)).click()
        assertTrue(device.wait(Until.gone(By.res("speedUp.customSheet")), 10_000L))
        wallet.waitUntil("the custom rate wasn't selected", timeoutMs = 15_000L) { isSelected("speedUp.option.custom") }

        continueToConfirm()
        swipeAndFinishFeeBump()
        val firstBump = assertReplaced(original)

        // the second bump takes the sheet's default
        openBumpSheetFromMenu(firstBump.txid)
        continueToConfirm()
        swipeAndFinishFeeBump()
        val secondBump = assertReplaced(firstBump)

        // the fee history lists the original, the first bump and the current fee
        openDetails(secondBump.txid)
        expandDetails()

        val history = device.scrollTo(By.res("transactionDetails.feeHistory"))
        assertFalse("the fee history should start collapsed", device.hasObject(By.res("transactionDetails.feeHistory.0")))
        history.click()

        (0 until 3).forEach { device.scrollTo(By.res("transactionDetails.feeHistory.$it"), timeoutMs = 15_000L) }
        assertFalse(device.hasObject(By.res("transactionDetails.feeHistory.3")))
        capture("details-fee-history")
    }

    @Test
    fun receivedPendingTransactionOffersNoBump() {
        fundTestWallet()
        val received = node.fundWallet(TEST_MNEMONIC, listOf(2 to RECEIVED_SATS), confirm = false).single()
        launchAndImportHotWallet()

        device.waitFor(transactionRow(received.outpoint.txid), timeoutMs = 120_000L).longClick()
        device.waitFor(By.res("transactionMenu.copyTxId"), timeoutMs = 10_000L)
        assertFalse("a received payment can't be sped up", device.hasObject(By.res("transactionMenu.speedUp")))
        capture("menu-received")

        device.waitFor(By.res("transactionMenu.details")).click()
        device.scrollTo(By.res("transactionDetails.showDetails"), timeoutMs = 30_000L)
        assertFalse(device.hasObject(By.res("transactionDetails.speedUp")))
        assertFalse(device.hasObject(By.res("transactionDetails.delayedBanner")))
    }

    // MARK: hardware wallet

    @Test
    fun hardwareWalletBumpSignsTheReplacement() {
        signer.requireHealthy()
        fundTestWallet()
        screens.importWatchOnlyWallet(signer, qr, FUNDED_SATS.sum())

        startSlowPayment(expecting = By.text("Import Signature"))
        signOnHardware()
        wallet.swipeToSend()
        val original = node.broadcasts().single()

        // bumping exports the replacement for the signer, which signs it like any draft
        openDetails(original.txid)
        device.waitFor(By.res("transactionDetails.speedUp"), timeoutMs = 30_000L).click()
        assertBumpSheetSelects("medium")
        device.scrollTo(By.res("speedUp.continue")).click()

        device.waitFor(By.text("Export Transaction"), timeoutMs = 30_000L)
        signOnHardware()
        assertConfirmInBumpMode()
        swipeAndFinishFeeBump()

        assertReplaced(original)
    }

    // MARK: fixture

    /** Funds external indexes 0 and 1 of the test mnemonic, each its own confirmed UTXO */
    private fun fundTestWallet() {
        node.reset()

        val outputs = node.fundWallet(TEST_MNEMONIC, FUNDED_SATS.mapIndexed { index, sats -> index to sats })
        node.mine()

        assertEquals(FUNDED_SATS, outputs.map { it.sats })
    }

    private fun launchAndImportHotWallet() {
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
    }

    // MARK: sending

    /**
     * Sends the test amount at Slow, which pays under Medium so the send can be sped up and counts
     * as delayed once it is late, and returns its broadcast
     */
    private fun sendSlowPayment(): FakeNodeBroadcast {
        wallet.waitForBalance(FUNDED_SATS.sum())
        startSlowPayment()
        wallet.swipeToSend()

        val original = node.broadcasts().single()
        device.waitFor(transactionRow(original.txid), timeoutMs = 60_000L)

        return original
    }

    /** Fills the send screen with the test amount at Slow and taps Next */
    private fun startSlowPayment(expecting: BySelector = By.res("sendFlowConfirm.swipeToSend")) {
        wallet.startSend()
        wallet.enterAmount(SEND_AMOUNT_BTC)
        wallet.dismissKeyboard()

        device.scrollTo(By.text("Change speed"), timeoutMs = 30_000L).click()
        device.waitFor(By.res("sendFlow.feeOption.slow"), timeoutMs = 15_000L).click()
        assertTrue(
            "picking a speed should close the fee sheet",
            device.wait(Until.gone(By.res("sendFlow.feeOption.slow")), 10_000L),
        )

        wallet.clickNext(expecting)
    }

    /** Shares the open draft with the signer and scans its signature back in */
    private fun signOnHardware() {
        val signed = signer.sign(screens.exportUnsignedPsbt("UR"), 1)

        screens.openSignatureScanner()
        qr.show(signed.ur)
    }

    // MARK: speed up

    private fun openBumpSheetFromMenu(txid: String) {
        device.waitFor(transactionRow(txid), timeoutMs = 60_000L).longClick()

        device.waitFor(By.res("transactionMenu.speedUp"), timeoutMs = 10_000L)
        assertTrue(device.hasObject(By.res("transactionMenu.copyAddress")))
        assertTrue(device.hasObject(By.res("transactionMenu.copyTxId")))
        assertTrue(device.hasObject(By.res("transactionMenu.details")))
        capture("menu-pending-send")

        device.waitFor(By.res("transactionMenu.speedUp")).click()
        device.waitFor(By.res("speedUp.sheet"), timeoutMs = 15_000L)
    }

    /** Waits for the Speed Up sheet's rates and checks which one it starts on */
    private fun assertBumpSheetSelects(speed: String) {
        val tag = "speedUp.option.$speed"
        device.waitFor(By.res(tag), timeoutMs = 30_000L)
        wallet.waitUntil("the Speed Up sheet should start on $speed", timeoutMs = 10_000L) { isSelected(tag) }

        assertTrue(device.hasObject(By.res("speedUp.currentFee")))

        // the sheet scrolls, and a short screen leaves its cost summary below the fold
        device.scrollTo(By.res("speedUp.newFee"))
        device.scrollTo(By.res("speedUp.additionalCost"))
    }

    // compose reports a selected row as checked unless its role is a tab
    private fun isSelected(tag: String): Boolean =
        device.hasObject(By.res(tag).selected(true)) || device.hasObject(By.res(tag).checked(true))

    private fun assertNote(
        tag: String,
        contains: String,
    ) {
        device.scrollTo(By.res(tag))
        val note = device.waitForText(By.res(tag)).orEmpty()
        assertTrue("$tag reads '$note'", note.contains(contains))
    }

    private fun continueToConfirm() {
        device.scrollTo(By.res("speedUp.continue"), timeoutMs = 30_000L)
        val label = device.waitForText(By.res("speedUp.continue")).orEmpty()
        assertTrue("unexpected continue label '$label'", label.startsWith("Speed Up for "))
        device.waitFor(By.res("speedUp.continue").enabled(true)).click()

        assertConfirmInBumpMode()
    }

    private fun assertConfirmInBumpMode() {
        device.waitFor(By.res("sendFlowConfirm.title"), timeoutMs = 30_000L)
        wallet.waitUntil("the confirm screen should be titled Speed Up", timeoutMs = 15_000L) {
            device.findText(By.res("sendFlowConfirm.title")) == "Speed Up"
        }

        device.waitFor(By.res("sendFlowConfirm.previousFee"), timeoutMs = 10_000L)
        assertTrue(device.hasObject(By.text("New Network Fee")))
        assertTrue(device.hasObject(By.text("Swipe to confirm new fee")))
    }

    /** Swipes the replacement out, then leaves the Sped Up screen for the wallet */
    private fun swipeAndFinishFeeBump() {
        val track = device.waitFor(By.res("sendFlowConfirm.swipeToSend")).visibleBounds
        device.swipe(track.left + 40, track.centerY(), track.right - 4, track.centerY(), 40)

        device.waitFor(By.res("spedUp.title"), timeoutMs = 60_000L)
        assertFalse("a bump shows Sped Up instead of the sent alert", device.hasObject(By.text("Transaction sent successfully!")))
        assertTrue(device.hasObject(By.res("spedUp.fee")))
        capture("sped-up")

        device.waitFor(By.res("spedUp.done")).click()
        device.waitFor(By.res("selectedWallet.balance"), timeoutMs = 30_000L)
    }

    /**
     * Checks a new broadcast took the place of [replaced], spending its inputs, and that the node
     * now holds it for them, returning it
     */
    private fun assertReplaced(replaced: FakeNodeBroadcast): FakeNodeBroadcast {
        val replacement = waitForReplacement(replaced)
        assertEquals("a bump must spend the coins it replaces", replaced.inputs.toSet(), replacement.inputs.toSet())
        assertTrue(replacement.outputs.any { it.address == EXTERNAL_ADDRESS })

        replaced.inputs.forEach {
            assertEquals("the node should hold the replacement", replacement.txid, node.spendingTxid(it))
        }

        device.waitFor(transactionRow(replacement.txid), timeoutMs = 60_000L)
        return replacement
    }

    /**
     * Polls the node for the broadcast that spends what [replaced] spent under another txid
     *
     * The node drops a replaced transaction from its broadcast log, so the log holds only the
     * latest version of a payment
     */
    private fun waitForReplacement(replaced: FakeNodeBroadcast): FakeNodeBroadcast {
        val deadline = System.currentTimeMillis() + 15_000L

        while (System.currentTimeMillis() < deadline) {
            node
                .broadcasts()
                .firstOrNull { it.txid != replaced.txid && it.inputs.any(replaced.inputs::contains) }
                ?.let { return it }

            Thread.sleep(500)
        }

        error("nothing replaced ${replaced.txid}: ${node.broadcasts()}")
    }

    // MARK: wallet screen

    private fun transactionRow(txid: String): BySelector = By.res("transactionRow.$txid")

    private fun openDetails(txid: String) {
        val row = transactionRow(txid)
        device.waitFor(row, timeoutMs = 60_000L).click()
        device.wait(Until.gone(row), 10_000L)

        device.scrollTo(By.res("transactionDetails.showDetails"), timeoutMs = 30_000L)
    }

    /** Opens the details' fee rows, which stay open for the wallet once shown */
    private fun expandDetails() {
        val showDetails = device.scrollTo(By.res("transactionDetails.showDetails"))
        if (showDetails.anyText() == "Show Details") showDetails.click()

        device.scrollTo(By.res("transactionDetails.networkFee"), timeoutMs = 15_000L)
    }

    private fun capture(name: String) = capture.capture(name)
}

/** Saves what the screen shows in light and dark, when a run asks for screenshots */
private class ScreenCapture(
    private val device: UiDevice,
) {
    private val directory = InstrumentationRegistry.getArguments().getString("coveScreenshotDir")

    fun capture(name: String) {
        val directory = directory ?: return

        device.executeShellCommand("mkdir -p $directory")
        save(name, directory, ColorSchemeSelection.LIGHT)
        save(name, directory, ColorSchemeSelection.DARK)
        setColorScheme(ColorSchemeSelection.SYSTEM)
    }

    // the app's own color scheme switches the theme in place, where the system's dark mode would
    // recreate the activity and close any open sheet
    private fun save(
        name: String,
        directory: String,
        colorScheme: ColorSchemeSelection,
    ) {
        setColorScheme(colorScheme)

        // the app redraws a moment after the scheme changes
        Thread.sleep(1_500)
        device.waitForIdle()

        val suffix = if (colorScheme == ColorSchemeSelection.DARK) "dark" else "light"
        val output = device.executeShellCommand("screencap -p $directory/$name-$suffix.png")
        check(output.isBlank()) { "couldn't save the $name screenshot: $output\n${device.hierarchy()}" }
    }

    private fun setColorScheme(colorScheme: ColorSchemeSelection) {
        InstrumentationRegistry.getInstrumentation().runOnMainSync {
            AppManager.getInstance().dispatch(AppAction.ChangeColorScheme(colorScheme))
        }
    }
}
