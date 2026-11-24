@file:Suppress("PackageNaming")

package org.bitcoinppl.cove.flows.SendFlow

import android.os.Process
import android.system.Os
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.filters.LargeTest
import androidx.test.platform.app.InstrumentationRegistry
import androidx.test.uiautomator.By
import androidx.test.uiautomator.UiDevice
import androidx.test.uiautomator.Until
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
import org.bitcoinppl.cove.test.fullLaunchDevice
import org.bitcoinppl.cove.test.hierarchy
import org.bitcoinppl.cove.test.launchFullApp
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
 * directory, OHTTP relay and receiver the UI test host runs, then checks what the app broadcast
 *
 * The host's relay reaches the app through the debug-only `COVE_UI_TEST_PAYJOIN_RELAY` seam, set
 * in process before each launch, and the receiver's coin and the wallet's are on cove-fake-node.
 * The relaunch case runs as two ordered stages, as `UtxoLockFullLaunchTest` does: the test
 * orchestrator gives every test method its own instrumentation run, which force-stops the app, so
 * stage two finds the payment its wallet saved in a fresh process
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

        const val SENT = "Transaction sent successfully!"
        const val SENT_WITHOUT_PAYJOIN = "Transaction sent successfully, without Payjoin."
        const val BROADCAST_ERROR = "fake node refused the broadcast"

        const val MARKER_FILE = "payjoin-send-full-launch-stage.json"
        const val MARKER_MAX_AGE_MS = 30 * 60 * 1_000L

        // the session waits ten minutes, so a fresh countdown reads 10:00 or a few seconds under it
        val FRESH_COUNTDOWN = Regex("""(10:00|9:[0-5]\d) left""")
        val COUNTDOWN = Regex("""(\d+):(\d\d) left""")
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
        val payment = startPayjoinPayment()

        // nothing goes out while the wallet waits for the receiver
        assertEquals("the payment was broadcast before the receiver answered", 0, node.broadcasts().size)

        // with the proposal's broadcast held, the card is gone and nothing claims the payment went out
        node.holdBroadcasts()
        receiver.respond()
        node.waitForHeldBroadcast()
        assertGoneWhileBroadcastHeld(WAITING_CARD_TAG, "the card stayed while the proposal went out")
        node.releaseBroadcasts()

        device.waitFor(By.text(SENT), timeoutMs = 60_000L)
        assertTrue("a joined payment must not say it went out without Payjoin", !device.hasObject(By.text(SENT_WITHOUT_PAYJOIN)))
        device.waitFor(By.text("OK")).click()

        // the one broadcast is the receiver's proposal, spending its coin alongside the wallet's
        val proposal = singleBroadcast()
        assertEquals(setOf(payment.walletCoin, payment.receiver.coin), proposal.inputs.toSet())
        assertTrue(
            "the proposal must still pay the receiver: ${proposal.outputs}",
            proposal.outputs.any { it.address == payment.receiver.address },
        )
        device.waitFor(By.res("selectedWallet.balance"), timeoutMs = 30_000L)
    }

    @Test
    fun sendWithoutPayjoinBroadcastsTheOriginalOnce() {
        val payment = startPayjoinPayment()

        // with the fallback's broadcast held, the card is gone and nothing claims the payment went out
        node.holdBroadcasts()
        device.waitFor(By.res(SEND_WITHOUT_TAG)).click()
        node.waitForHeldBroadcast()
        assertGoneWhileBroadcastHeld(WAITING_CARD_TAG, "the card stayed while the fallback went out")
        node.releaseBroadcasts()

        device.waitFor(By.text(SENT_WITHOUT_PAYJOIN), timeoutMs = 60_000L)
        device.waitFor(By.text("OK")).click()

        assertIsTheOriginal(singleBroadcast(), payment)
    }

    @Test
    fun failedFallbackBroadcastShowsTheNodeError() {
        startPayjoinPayment()

        node.failBroadcasts(BROADCAST_ERROR)
        device.waitFor(By.res(SEND_WITHOUT_TAG)).click()

        device.waitFor(By.textContains(BROADCAST_ERROR), timeoutMs = 60_000L)
        assertTrue("a refused payment must not report sent", !device.hasObject(By.text(SENT_WITHOUT_PAYJOIN)))
        assertEquals("the node logged a broadcast it refused", 0, node.broadcasts().size)
        assertEquals("the refused fallback was sent more than once", 1, node.broadcastRequestCount())
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

    /**
     * Imports the funded wallet, pays a new receiver's pj URI and waits on the confirm screen until
     * it shows the wait for the receiver
     */
    private fun startPayjoinPayment(): Payment {
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

        // the receiver's funding mines a block, so it comes after the wallet's scan has settled
        val session = receiver.open(PAYMENT_SATS)

        // the URI carries the amount, so the amount field fills in on its own
        device.waitFor(By.text("Send")).click()
        wallet.enterPaymentRequest(session.pjUri, session.address)
        wallet.clickNext()
        assertTrue("the confirm screen waits before the payment was sent", !device.hasObject(By.res(WAITING_CARD_TAG)))
        wallet.swipeToConfirm()

        device.scrollTo(By.res(WAITING_CARD_TAG), timeoutMs = 30_000L)
        device.waitFor(By.text("Waiting for the receiver"))
        val countdown = device.waitForText(By.res(COUNTDOWN_TAG)).orEmpty()
        assertTrue("unexpected countdown: $countdown", FRESH_COUNTDOWN.matches(countdown))
        assertTrue(device.waitFor(By.res(SEND_WITHOUT_TAG)).isEnabled)
        assertTrue("a Payjoin payment must not report sent while it waits", !device.hasObject(By.text(SENT)))

        return Payment(walletCoin, session)
    }

    /** Waits for [tag] to go while a held broadcast keeps the payment unsent, then checks nothing reported it sent */
    private fun assertGoneWhileBroadcastHeld(
        tag: String,
        message: String,
    ) {
        assertTrue("$message\n${device.hierarchy()}", device.wait(Until.gone(By.res(tag)), 60_000L))
        assertTrue(
            "the payment was reported sent before its broadcast answered",
            !device.hasObject(By.text(SENT)) && !device.hasObject(By.text(SENT_WITHOUT_PAYJOIN)),
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

    private fun assertIsTheOriginal(
        broadcast: FakeNodeBroadcast,
        payment: Payment,
    ) {
        assertEquals("the fallback must spend only the wallet's coin", listOf(payment.walletCoin), broadcast.inputs)
        assertTrue(
            "the fallback must pay the receiver the requested amount: ${broadcast.outputs}",
            broadcast.outputs.any { it.address == payment.receiver.address && it.sats == PAYMENT_SATS },
        )
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
