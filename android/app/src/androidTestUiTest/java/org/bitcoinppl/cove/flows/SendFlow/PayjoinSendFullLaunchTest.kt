@file:Suppress("PackageNaming")

package org.bitcoinppl.cove.flows.SendFlow

import android.os.Process
import android.system.Os
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.filters.LargeTest
import androidx.test.platform.app.InstrumentationRegistry
import androidx.test.uiautomator.By
import androidx.test.uiautomator.StaleObjectException
import androidx.test.uiautomator.UiDevice
import androidx.test.uiautomator.Until
import org.bitcoinppl.cove.flows.settings.clickSettled
import org.bitcoinppl.cove.flows.settings.clickText
import org.bitcoinppl.cove.test.AndroidDeviceStayAwakeRule
import org.bitcoinppl.cove.test.FAKE_NODE_ESPLORA_URL
import org.bitcoinppl.cove.test.FakeNodeBroadcast
import org.bitcoinppl.cove.test.FakeNodeControl
import org.bitcoinppl.cove.test.FakeNodeOutPoint
import org.bitcoinppl.cove.test.FullLaunchOnboardingRobot
import org.bitcoinppl.cove.test.FullLaunchStartupRobot
import org.bitcoinppl.cove.test.FullLaunchWalletRobot
import org.bitcoinppl.cove.test.HostControlHttp
import org.bitcoinppl.cove.test.TEST_MNEMONIC
import org.bitcoinppl.cove.test.anyText
import org.bitcoinppl.cove.test.fullLaunchDevice
import org.bitcoinppl.cove.test.hierarchy
import org.bitcoinppl.cove.test.launchFullApp
import org.bitcoinppl.cove.test.parseSats
import org.bitcoinppl.cove.test.pressBackInApp
import org.bitcoinppl.cove.test.scrollTo
import org.bitcoinppl.cove.test.waitFor
import org.bitcoinppl.cove.test.waitForText
import org.json.JSONObject
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotEquals
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.FixMethodOrder
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.junit.runners.MethodSorters
import java.io.File

/**
 * Pays a BIP 77 receiver's pj URI from the real send flow, with a real Payjoin session against the
 * directory, OHTTP relay and receiver the UI test host runs, then checks what the app broadcast and
 * the fees the confirm screen, success alert and transaction details show
 *
 * The host's relay reaches the app through the debug-only `COVE_UI_TEST_PAYJOIN_RELAY` seam, set
 * in process before each launch, and the receiver's coin and the wallet's are on cove-fake-node.
 * The relaunch case runs as two ordered stages, as `UtxoLockFullLaunchTest` does: the test
 * orchestrator gives every test method its own instrumentation run, which force-stops the app, so
 * stage two finds the payment its wallet saved in a fresh process. Every other test's name sorts
 * before the stages, so nothing runs between them
 *
 * Drives the app with UiAutomator, since a Compose test environment stalls a cold launch at
 * bootstrap. Runs in the uiTest flavour with the host up and forwarded, which
 * `just android-instrumented-ci` does
 */
@LargeTest
@RunWith(AndroidJUnit4::class)
@FixMethodOrder(MethodSorters.NAME_ASCENDING)
class PayjoinSendFullLaunchTest {
    private companion object {
        const val WALLET_SATS = 100_000L
        const val PAYMENT_SATS = 20_000L
        const val RELAY_URL = "http://127.0.0.1:39012"

        const val WAITING_CARD_TAG = "payjoin.waitingCard"
        const val PENDING_BANNER_TAG = "payjoin.pendingBanner"
        const val COUNTDOWN_TAG = "payjoin.countdown"
        const val SEND_WITHOUT_TAG = "payjoin.sendWithoutPayjoin"

        const val PAYJOIN_TOGGLE_TAG = "sendConfirm.payjoinToggle"
        const val NETWORK_FEE_TAG = "sendConfirm.networkFee"
        const val FEE_HELP_TAG = "sendConfirm.payjoinFeeHelp"
        const val TOTAL_TAG = "sendConfirm.totalSpending"
        const val SUCCESS_MESSAGE_TAG = "sendConfirm.successMessage"

        const val SHOW_DETAILS_TAG = "transactionDetails.showDetails"
        const val DETAILS_NETWORK_FEE_TAG = "transactionDetails.networkFee"

        const val SETTINGS_PAYJOIN_TAG = "settings.payjoin"
        const val UP_TO_ONE_INPUT_TAG = "payjoinSettings.feeHelp.upToOneInput"
        const val DONT_HELP_TAG = "payjoinSettings.feeHelp.dontHelp"

        // the unconfirmed sent row's label while the wallet has no label of its own for it
        const val SENDING_ROW = "Sending"

        const val SENT = "Transaction sent successfully!"
        const val BROADCAST_ERROR = "fake node refused the broadcast"
        const val UP_TO = "up to "

        // the texts with amounts, each amount written as AMOUNT_SLOT, so a check reads the wording
        // apart from the figures
        const val AMOUNT_SLOT = "#"
        const val FEE_HELP_LINE = "+ up to # with Payjoin"
        const val SENT_WITH_FEE_HELP = "Sent with Payjoin. You paid # in fees, including # toward the receiver's coin."
        const val SENT_WITH_FEE = "Sent with Payjoin. You paid # in fees."
        const val SENT_WITHOUT_PAYJOIN = "Sent without Payjoin. You paid # in fees."

        const val MARKER_FILE = "payjoin-send-full-launch-stage.json"
        const val MARKER_MAX_AGE_MS = 30 * 60 * 1_000L

        // the session waits ten minutes, so a fresh countdown reads 10:00 or a few seconds under it
        val FRESH_COUNTDOWN = Regex("""(10:00|9:[0-5]\d) left""")
        val COUNTDOWN = Regex("""(\d+):(\d\d) left""")

        // an amount as the app formats it in either unit, such as 0.00000141 BTC or 1,410 SATS
        val AMOUNT = Regex("""\d[\d,]*(?:\.\d+)? (?:BTC|SATS)""")
    }

    private data class Payment(
        val walletCoin: FakeNodeOutPoint,
        val receiver: PayjoinReceiverSession,
    )

    /** What stage 1 left for stage 2: its payment, and the countdown it read at [readAtMs] */
    private data class Stage1(
        val payment: Payment,
        val secondsLeft: Int,
        val readAtMs: Long,
    )

    /** The confirm screen's figures in sats, with the texts they were read from */
    private data class ConfirmFigures(
        val fee: Long,
        /** The most the receiver may take from the change, when the fee help line shows */
        val feeHelp: Long?,
        val total: Long,
        val feeHelpText: String?,
        val totalText: String,
    )

    @get:Rule
    val stayAwake = AndroidDeviceStayAwakeRule()

    private lateinit var device: UiDevice
    private val node = FakeNodeControl()
    private val receiver = PayjoinReceiverControl()
    private val wallet by lazy { FullLaunchWalletRobot(device) }
    private var previousShowImeWithHardKeyboard: String? = null

    private val markerFile: File
        get() = File(InstrumentationRegistry.getInstrumentation().targetContext.noBackupFilesDir, MARKER_FILE)

    @Before
    fun prepareDevice() {
        device = fullLaunchDevice()
        previousShowImeWithHardKeyboard = device.executeShellCommand("settings get secure show_ime_with_hard_keyboard").trim()

        // the onboarding robot presses Back while the soft keyboard shows, which can leave the
        // import screen if the keyboard is already closing, so it stays hidden
        device.executeShellCommand("settings put secure show_ime_with_hard_keyboard 0")

        // each test is a new process, and the wallet reads the relay when it loads, so both are
        // set before every launch
        Os.setenv("COVE_FEE_URL", "$FAKE_NODE_ESPLORA_URL/api/v1/fees/recommended", true)
        Os.setenv("COVE_UI_TEST_PAYJOIN_RELAY", RELAY_URL, true)
    }

    @After
    fun restoreDevice() {
        when (val previous = previousShowImeWithHardKeyboard) {
            null, "null" -> device.executeShellCommand("settings delete secure show_ime_with_hard_keyboard")
            else -> device.executeShellCommand("settings put secure show_ime_with_hard_keyboard $previous")
        }
    }

    @Test
    fun receiverJoinsAPaymentToItsPjUri() {
        val payment = openPayjoinConfirm(importFundedWallet())

        // the switch starts on, and the payment offers the receiver some of its change, so the fee
        // and total show the most the payment can cost
        assertPayjoinSwitch(on = true)
        val figures = readConfirmFigures()
        val feeHelp = checkNotNull(figures.feeHelp) { "the confirm screen has no fee help line: $figures\n${device.hierarchy()}" }
        assertEquals("unexpected fee help line: $figures", FEE_HELP_LINE, figures.feeHelpText?.replace(AMOUNT, AMOUNT_SLOT))
        assertTrue("the fee help must offer more than nothing: $figures", feeHelp > 0)
        assertTrue("the total must read \"$UP_TO\" with fee help: $figures", figures.totalText.startsWith(UP_TO))
        assertEquals(
            "the total must be the payment, the fee and the fee help: $figures",
            PAYMENT_SATS + figures.fee + feeHelp,
            figures.total,
        )

        swipeAndWaitForReceiver()

        // nothing goes out while the wallet waits for the receiver
        assertEquals("the payment was broadcast before the receiver answered", 0, node.broadcasts().size)

        // with the proposal's broadcast held, the card is gone and nothing claims the payment went out
        node.holdBroadcasts()
        receiver.respond()
        node.waitForHeldBroadcast()
        assertGoneWhileBroadcastHeld(WAITING_CARD_TAG, "the card stayed while the proposal went out")
        node.releaseBroadcasts()

        val (feePaid, feeHelpTaken) = amountsIn(successMessage(), SENT_WITH_FEE_HELP)
        device.waitFor(By.text("OK")).click()

        // the one broadcast is the receiver's proposal, spending its coin alongside the wallet's
        val proposal = singleBroadcast()
        assertEquals(setOf(payment.walletCoin, payment.receiver.coin), proposal.inputs.toSet())
        assertTrue(
            "the proposal must still pay the receiver: ${proposal.outputs}",
            proposal.outputs.any { it.address == payment.receiver.address },
        )

        // the alert's fee is all the wallet spent beyond the payment, and the receiver took its
        // part of it from the change, within the offer
        val change = changeOf(proposal, payment)
        assertEquals(
            "the fee paid must be what the wallet spent beyond the payment: paid $feePaid, change $change",
            WALLET_SATS - PAYMENT_SATS - change,
            feePaid,
        )

        assertEquals(
            "the receiver's part must be the fee paid beyond the confirm screen's ${figures.fee}: paid $feePaid, part $feeHelpTaken",
            feePaid - figures.fee,
            feeHelpTaken,
        )

        assertTrue(
            "the receiver must take some of the $feeHelp offered and no more: took $feeHelpTaken",
            feeHelpTaken in 1..feeHelp,
        )

        device.waitFor(By.res("selectedWallet.balance"), timeoutMs = 30_000L)
        assertEquals("transaction details must show the fee the Payjoin paid", feePaid, sentTransactionFee())
    }

    @Test
    fun sendWithoutPayjoinBroadcastsTheOriginalOnce() {
        val payment = openPayjoinConfirm(importFundedWallet())
        val figures = readConfirmFigures()
        swipeAndWaitForReceiver()

        // with the fallback's broadcast held, the card is gone and nothing claims the payment went out
        node.holdBroadcasts()
        device.waitFor(By.res(SEND_WITHOUT_TAG)).click()
        node.waitForHeldBroadcast()
        assertGoneWhileBroadcastHeld(WAITING_CARD_TAG, "the card stayed while the fallback went out")
        node.releaseBroadcasts()

        val feePaid = amountsIn(successMessage(), SENT_WITHOUT_PAYJOIN).single()
        device.waitFor(By.text("OK")).click()

        // the Original pays exactly the fee the confirm screen showed
        val original = singleBroadcast()
        assertIsTheOriginal(original, payment)
        assertEquals("the fallback must pay the confirm screen's fee: $figures", figures.fee, feePaid)

        val change = changeOf(original, payment)
        assertEquals(
            "the confirm screen's fee must be what the Original spent beyond the payment: $figures, change $change",
            WALLET_SATS - PAYMENT_SATS - change,
            figures.fee,
        )
    }

    @Test
    fun failedFallbackBroadcastShowsTheNodeError() {
        startPayjoinPayment()

        node.failBroadcasts(BROADCAST_ERROR)
        device.waitFor(By.res(SEND_WITHOUT_TAG)).click()

        device.waitFor(By.textContains(BROADCAST_ERROR), timeoutMs = 60_000L)
        assertTrue("a refused payment must not report sent", !device.hasObject(By.res(SUCCESS_MESSAGE_TAG)))
        assertEquals("the node logged a broadcast it refused", 0, node.broadcasts().size)
        assertEquals("the refused fallback was sent more than once", 1, node.broadcastRequestCount())
    }

    @Test
    fun payjoinSwitchOffSendsANormalPayment() {
        val payment = openPayjoinConfirm(importFundedWallet())

        // with the switch off the receiver can't join, so it can take nothing and the total is exact
        assertPayjoinSwitch(on = true)
        device.scrollTo(By.res(PAYJOIN_TOGGLE_TAG)).click()
        assertPayjoinSwitch(on = false)
        assertTrue(
            "the fee help line stayed with the switch off\n${device.hierarchy()}",
            device.wait(Until.gone(By.res(FEE_HELP_TAG)), 5_000L),
        )

        val figures = readConfirmFigures()
        assertTrue("the total must be exact with the switch off: $figures", !figures.totalText.contains(UP_TO.trim()))
        assertEquals("the total must be the payment and the fee: $figures", PAYMENT_SATS + figures.fee, figures.total)

        wallet.swipeToConfirm()
        assertEquals("a payment with the switch off must report a normal send", SENT, successMessageWithoutWaiting())
        device.waitFor(By.text("OK")).click()

        // the one broadcast is the payment as built, with no receiver coin
        val broadcast = singleBroadcast()
        assertIsTheOriginal(broadcast, payment)

        val change = changeOf(broadcast, payment)
        assertEquals(
            "the payment must pay the confirm screen's fee: $figures, change $change",
            WALLET_SATS - PAYMENT_SATS - change,
            figures.fee,
        )
    }

    @Test
    fun dontHelpOffersTheReceiverNothing() {
        val walletCoin = importFundedWallet()
        chooseDontHelp()
        val payment = openPayjoinConfirm(walletCoin)

        // the payment is still a Payjoin, but offers nothing, so the fee and total are exact
        assertPayjoinSwitch(on = true)
        val figures = readConfirmFigures()
        assertEquals("Don't help must show no fee help line: $figures", null, figures.feeHelpText)
        assertTrue("the total must be exact with Don't help: $figures", !figures.totalText.contains(UP_TO.trim()))
        assertEquals("the total must be the payment and the fee: $figures", PAYMENT_SATS + figures.fee, figures.total)

        swipeAndWaitForReceiver()
        receiver.respond()

        val feePaid = amountsIn(successMessage(), SENT_WITH_FEE).single()
        device.waitFor(By.text("OK")).click()

        // the receiver pays for its own coin, so the wallet pays just the confirm screen's fee
        assertEquals("with nothing offered the wallet must pay the confirm screen's fee: $figures", figures.fee, feePaid)

        val proposal = singleBroadcast()
        assertEquals(
            "the proposal must spend the receiver's coin and the wallet's",
            setOf(payment.walletCoin, payment.receiver.coin),
            proposal.inputs.toSet(),
        )

        val change = changeOf(proposal, payment)
        assertEquals(
            "the fee paid must be what the wallet spent beyond the payment: paid $feePaid, change $change",
            WALLET_SATS - PAYMENT_SATS - change,
            feePaid,
        )
    }

    @Test
    fun stage1PaymentWaitsForTheReceiverBeforeARelaunch() {
        markerFile.delete()
        val payment = startPayjoinPayment()

        // the app has posted the payment to the directory, so its session survives the relaunch
        receiver.waitForOriginal()
        writeMarker(payment, secondsLeft())
    }

    @Test
    fun stage2WalletScreenResumesTheWaitingPayment() {
        val stage1 = consumeMarker()
        val payment = stage1.payment

        launchFullApp(resetData = false)
        device.waitFor(By.res("selectedWallet.balance"), timeoutMs = 60_000L)
        device.waitFor(By.res(PENDING_BANNER_TAG), timeoutMs = 30_000L)
        device.waitFor(By.text("Payjoin payment waiting for the receiver"))

        // the resumed session keeps its deadline, so the countdown lost just the time the relaunch
        // took: a restarted ten minutes would read more, and a deadline moved minutes earlier
        // less. The lower slack is wide because a loaded host once read a countdown 13 s behind
        // its redraw
        val secondsLeftAfter = secondsLeft()
        val expected = stage1.secondsLeft - ((System.currentTimeMillis() - stage1.readAtMs) / 1_000L).toInt()
        assertTrue(
            "the deadline moved over the relaunch: ${stage1.secondsLeft} s, then $secondsLeftAfter s where $expected s was due",
            secondsLeftAfter in (expected - 30)..(expected + 2),
        )
        assertEquals("the relaunch broadcast the payment", 0, node.broadcasts().size)

        // while the fallback's broadcast is held the banner names it, and goes once it is out
        node.holdBroadcasts()
        device.waitFor(By.res(SEND_WITHOUT_TAG)).click()
        node.waitForHeldBroadcast()
        device.waitFor(By.text("Sending without Payjoin…"), timeoutMs = 30_000L)
        node.releaseBroadcasts()
        assertTrue(
            "the banner stayed after the payment went out\n${device.hierarchy()}",
            device.wait(Until.gone(By.res(PENDING_BANNER_TAG)), 60_000L),
        )

        assertIsTheOriginal(singleBroadcast(), payment)
    }

    // MARK: payment

    /** Imports the funded wallet and returns its coin once the wallet's first scan has settled */
    private fun importFundedWallet(): FakeNodeOutPoint {
        node.reset()
        val walletCoin = node.fundWallet(TEST_MNEMONIC, listOf(0 to WALLET_SATS)).single().outpoint
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
        wallet.waitForBalance(WALLET_SATS)

        return walletCoin
    }

    /** Opens a new receiver's session and enters its pj URI, stopping on the confirm screen before the swipe */
    private fun openPayjoinConfirm(walletCoin: FakeNodeOutPoint): Payment {
        // the receiver's funding mines a block, so it comes after the wallet's scan has settled
        val session = receiver.open(PAYMENT_SATS)

        // the URI carries the amount, so the amount field fills in on its own
        device.waitFor(By.text("Send")).click()
        wallet.enterPaymentRequest(session.pjUri, session.address)
        wallet.clickNext()
        assertTrue("the confirm screen waits before the payment was sent", !device.hasObject(By.res(WAITING_CARD_TAG)))

        return Payment(walletCoin, session)
    }

    /** Swipes to send and waits on the confirm screen until it shows the wait for the receiver */
    private fun swipeAndWaitForReceiver() {
        wallet.swipeToConfirm()

        device.scrollTo(By.res(WAITING_CARD_TAG), timeoutMs = 30_000L)
        device.waitFor(By.text("Waiting for the receiver"))
        val countdown = device.waitForText(By.res(COUNTDOWN_TAG)).orEmpty()
        assertTrue("unexpected countdown: $countdown", FRESH_COUNTDOWN.matches(countdown))
        assertTrue(device.waitFor(By.res(SEND_WITHOUT_TAG)).isEnabled)
        assertTrue("a Payjoin payment must not report sent while it waits", !device.hasObject(By.res(SUCCESS_MESSAGE_TAG)))
    }

    /** Imports the funded wallet, pays a new receiver's pj URI and waits for the receiver */
    private fun startPayjoinPayment(): Payment {
        val payment = openPayjoinConfirm(importFundedWallet())
        swipeAndWaitForReceiver()

        return payment
    }

    private fun assertPayjoinSwitch(on: Boolean) {
        device.scrollTo(By.res(PAYJOIN_TOGGLE_TAG))
        assertTrue(
            "expected the Payjoin switch to be ${if (on) "on" else "off"}\n${device.hierarchy()}",
            device.wait(Until.hasObject(By.res(PAYJOIN_TOGGLE_TAG).checked(on)), 5_000L),
        )
    }

    /** Reads the confirm screen's fee, its fee help line when it shows one, and its total */
    private fun readConfirmFigures(): ConfirmFigures {
        device.scrollTo(By.res(NETWORK_FEE_TAG))
        val feeText = device.waitForText(By.res(NETWORK_FEE_TAG)).orEmpty()

        // the fee help line sits under the fee, so it is on screen with it whenever it shows
        val feeHelpText = if (device.hasObject(By.res(FEE_HELP_TAG))) device.waitForText(By.res(FEE_HELP_TAG)) else null

        device.scrollTo(By.res(TOTAL_TAG))
        val totalText = device.waitForText(By.res(TOTAL_TAG)).orEmpty()

        return ConfirmFigures(
            fee = singleAmount(feeText, "network fee"),
            feeHelp = feeHelpText?.let { singleAmount(it, "fee help line") },
            total = singleAmount(totalText, "total"),
            feeHelpText = feeHelpText,
            totalText = totalText,
        )
    }

    /** Waits for the success alert and returns its message */
    private fun successMessage(): String {
        device.waitFor(By.res(SUCCESS_MESSAGE_TAG), timeoutMs = 60_000L)

        return device.waitForText(By.res(SUCCESS_MESSAGE_TAG)).orEmpty()
    }

    /** Waits for the success alert, failing if the confirm screen waits for the receiver first */
    private fun successMessageWithoutWaiting(): String {
        val deadline = System.currentTimeMillis() + 60_000L

        while (System.currentTimeMillis() < deadline) {
            assertTrue(
                "the payment waited for the receiver with the switch off\n${device.hierarchy()}",
                !device.hasObject(By.res(WAITING_CARD_TAG)),
            )

            if (device.hasObject(By.res(SUCCESS_MESSAGE_TAG))) return successMessage()

            Thread.sleep(100)
        }

        error("Timed out waiting for the success alert\n${device.hierarchy()}")
    }

    /** Checks [text] reads as [shape] apart from its amounts, and returns those amounts in sats */
    private fun amountsIn(
        text: String,
        shape: String,
    ): List<Long> {
        assertEquals("unexpected text \"$text\"", shape, text.replace(AMOUNT, AMOUNT_SLOT))
        return amounts(text)
    }

    /** Every amount in [text], in sats */
    private fun amounts(text: String): List<Long> =
        AMOUNT
            .findAll(text)
            .map { match -> parseSats(match.value) ?: error("unreadable amount \"${match.value}\" in \"$text\"") }
            .toList()

    private fun singleAmount(
        text: String,
        what: String,
    ): Long = amounts(text).singleOrNull() ?: error("expected one amount in the $what, read \"$text\"\n${device.hierarchy()}")

    /** Waits for [tag] to go while a held broadcast keeps the payment unsent, then checks nothing reported it sent */
    private fun assertGoneWhileBroadcastHeld(
        tag: String,
        message: String,
    ) {
        assertTrue("$message\n${device.hierarchy()}", device.wait(Until.gone(By.res(tag)), 60_000L))
        assertTrue(
            "the payment was reported sent before its broadcast answered",
            !device.hasObject(By.res(SUCCESS_MESSAGE_TAG)),
        )
    }

    private fun secondsLeft(): Int {
        val text = device.waitForText(By.res(COUNTDOWN_TAG)).orEmpty()
        val match = COUNTDOWN.matchEntire(text) ?: error("unreadable countdown: $text\n${device.hierarchy()}")
        val (minutes, seconds) = match.destructured

        return minutes.toInt() * 60 + seconds.toInt()
    }

    private fun singleBroadcast(): FakeNodeBroadcast {
        val deadline = System.currentTimeMillis() + 15_000L
        var broadcasts = node.broadcasts()
        while (broadcasts.isEmpty() && System.currentTimeMillis() < deadline) {
            Thread.sleep(500)
            broadcasts = node.broadcasts()
        }

        assertEquals("expected exactly one broadcast: $broadcasts", 1, broadcasts.size)
        assertEquals("the payment was broadcast more than once", 1, node.broadcastRequestCount())
        return broadcasts.single()
    }

    /** Checks [broadcast] is the payment as the wallet built it, with no receiver coin */
    private fun assertIsTheOriginal(
        broadcast: FakeNodeBroadcast,
        payment: Payment,
    ) {
        assertEquals("the payment must spend only the wallet's coin", listOf(payment.walletCoin), broadcast.inputs)
        assertTrue(
            "the payment must pay the receiver the requested amount: ${broadcast.outputs}",
            broadcast.outputs.any { it.address == payment.receiver.address && it.sats == PAYMENT_SATS },
        )
    }

    // the wallet's change is the one output that doesn't pay the receiver
    private fun changeOf(
        broadcast: FakeNodeBroadcast,
        payment: Payment,
    ): Long {
        val change = broadcast.outputs.filter { it.address != payment.receiver.address }
        assertEquals("expected one change output: ${broadcast.outputs}", 1, change.size)

        return change.single().sats
    }

    // MARK: transaction details

    /** Opens the unconfirmed sent transaction from the wallet screen and reads the fee its details show */
    private fun sentTransactionFee(): Long {
        // the row appears once the wallet has the transaction, and reads Sending until it confirms
        assertTrue(
            "the wallet never listed the sent transaction\n${device.hierarchy()}",
            device.wait(Until.hasObject(By.text(SENDING_ROW)), 60_000L),
        )

        device.scrollTo(By.text(SENDING_ROW)).click()
        device.waitFor(By.res(SHOW_DETAILS_TAG), timeoutMs = 30_000L)
        expandTransactionDetails()

        device.scrollTo(By.res(DETAILS_NETWORK_FEE_TAG))
        val feeText = device.waitForText(By.res(DETAILS_NETWORK_FEE_TAG)).orEmpty()

        return singleAmount(feeText, "transaction details' network fee")
    }

    // expansion is saved in the wallet metadata, so the details may already show expanded
    private fun expandTransactionDetails() {
        val deadline = System.currentTimeMillis() + 20_000L

        while (System.currentTimeMillis() < deadline) {
            device.waitForIdle()
            if (device.hasObject(By.res(DETAILS_NETWORK_FEE_TAG))) return

            try {
                // the toggle sits below the details, past the fold on a small phone
                val toggle = device.scrollTo(By.res(SHOW_DETAILS_TAG), timeoutMs = deadline - System.currentTimeMillis())

                when (toggle.anyText()) {
                    // the fee is in the expanded rows above, which the caller scrolls to
                    "Hide Details" -> {
                        return
                    }

                    "Show Details" -> {
                        toggle.click()
                        device.wait(Until.hasObject(By.text("Hide Details")), 5_000L)
                    }

                    else -> {
                        Thread.sleep(250)
                    }
                }
            } catch (_: StaleObjectException) {
                // the details recompose as the transaction loads; find the toggle again
            }
        }

        error("transaction details didn't expand\n${device.hierarchy()}")
    }

    // MARK: settings

    /** Chooses Don't help in Settings > Payjoin, reached from the wallet's sidebar, and returns to the wallet */
    private fun chooseDontHelp() {
        device.clickSettled(By.desc("Menu"), "the wallet's menu button")

        // the sidebar slides in, so the row is tapped only once it has stopped moving
        device.clickText("Settings")
        device.scrollTo(By.res(SETTINGS_PAYJOIN_TAG)).click()

        // compose reports a radio button's selection to accessibility as checked, not selected
        device.waitFor(By.res(UP_TO_ONE_INPUT_TAG).checked(true))
        device.waitFor(By.res(DONT_HELP_TAG)).click()
        assertTrue(
            "Don't help wasn't checked after a tap\n${device.hierarchy()}",
            device.wait(Until.hasObject(By.res(DONT_HELP_TAG).checked(true)), 5_000L),
        )

        assertTrue(
            "Up to one input stayed checked beside Don't help\n${device.hierarchy()}",
            device.hasObject(By.res(UP_TO_ONE_INPUT_TAG).checked(false)),
        )

        // Settings sits on the wallet's route, so Back twice returns to the wallet
        device.pressBackInApp()
        device.waitFor(By.res(SETTINGS_PAYJOIN_TAG))
        device.pressBackInApp()
        device.waitFor(By.res("selectedWallet.balance"), timeoutMs = 30_000L)
    }

    // MARK: stages

    private fun writeMarker(
        payment: Payment,
        secondsLeft: Int,
    ) {
        val marker =
            JSONObject()
                .put("pid", Process.myPid())
                .put("createdAt", System.currentTimeMillis())
                .put("walletCoin", payment.walletCoin.toString())
                .put("receiverAddress", payment.receiver.address)
                .put("receiverCoin", payment.receiver.coin.toString())
                .put("pjUri", payment.receiver.pjUri)
                .put("secondsLeft", secondsLeft)

        markerFile.writeText(marker.toString())
    }

    private fun consumeMarker(): Stage1 {
        assertTrue("stage 2 needs stage 1 of this class to have just passed; run the whole class", markerFile.exists())

        val marker = JSONObject(markerFile.readText())
        markerFile.delete()

        assertTrue(
            "the stage 1 marker is stale; run the whole class",
            System.currentTimeMillis() - marker.getLong("createdAt") < MARKER_MAX_AGE_MS,
        )
        assertNotEquals(
            "stage 2 must run in a fresh process, which the test orchestrator provides",
            marker.getInt("pid"),
            Process.myPid(),
        )

        val receiver =
            PayjoinReceiverSession(
                pjUri = marker.getString("pjUri"),
                address = marker.getString("receiverAddress"),
                coin = FakeNodeOutPoint.parse(marker.getString("receiverCoin")),
            )
        val payment = Payment(FakeNodeOutPoint.parse(marker.getString("walletCoin")), receiver)

        return Stage1(payment, marker.getInt("secondsLeft"), marker.getLong("createdAt"))
    }
}

/** A receiver session the UI test host opened, and the coin it adds when it joins */
private data class PayjoinReceiverSession(
    val pjUri: String,
    /** The address the pj URI pays */
    val address: String,
    val coin: FakeNodeOutPoint,
)

/** The Payjoin receiver the UI test host plays (`rust/xtask/src/ui_test_host/payjoin.rs`) */
private class PayjoinReceiverControl(
    host: String = "127.0.0.1",
    port: Int = 39_010,
) {
    private val http =
        HostControlHttp(host, port) {
            "the UI test host at $host:$port is unreachable; run the test through " +
                "`just android-instrumented-ci` or `cargo xtask ui-test-host run --android -- <command>`"
        }

    /** Funds the receiver's coin on the fake node and opens a session asking for [amountSats] */
    fun open(amountSats: Long): PayjoinReceiverSession {
        val json = JSONObject(http.request("POST", "/payjoin/receiver", JSONObject().put("amount_sats", amountSats)))

        return PayjoinReceiverSession(
            pjUri = json.getString("pj_uri"),
            address = json.getString("address"),
            coin = FakeNodeOutPoint.parse(json.getString("coin")),
        )
    }

    /** Waits until the app's Original PSBT reaches the directory, leaving it unanswered */
    fun waitForOriginal() {
        http.request("POST", "/payjoin/original")
    }

    /** Joins the app's payment with the receiver's coin and sends the proposal back */
    fun respond() {
        http.request("POST", "/payjoin/respond")
    }
}
