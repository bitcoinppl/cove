package org.bitcoinppl.cove.flows.keyteleport

import android.os.Build
import android.os.SystemClock
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.filters.LargeTest
import androidx.test.platform.app.InstrumentationRegistry
import androidx.test.uiautomator.By
import androidx.test.uiautomator.StaleObjectException
import androidx.test.uiautomator.UiDevice
import androidx.test.uiautomator.Until
import org.bitcoinppl.cove.AppManager
import org.bitcoinppl.cove.test.AndroidDeviceStayAwakeRule
import org.bitcoinppl.cove.test.FullLaunchOnboardingRobot
import org.bitcoinppl.cove.test.FullLaunchStartupRobot
import org.bitcoinppl.cove.test.FullLaunchWalletRobot
import org.bitcoinppl.cove.test.HostControlHttp
import org.bitcoinppl.cove.test.QrScript
import org.bitcoinppl.cove.test.awaitOnMain
import org.bitcoinppl.cove.test.fullLaunchDevice
import org.bitcoinppl.cove.test.hierarchy
import org.bitcoinppl.cove.test.launchFullApp
import org.bitcoinppl.cove.test.waitFor
import org.bitcoinppl.cove_core.Route
import org.bitcoinppl.cove_core.RouteFactory
import org.json.JSONArray
import org.json.JSONObject
import org.junit.After
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Assume.assumeTrue
import org.junit.Before
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import java.util.concurrent.atomic.AtomicBoolean

/**
 * Receives a KeyTeleport response through the wallet screen's QR scanner rather than the receive
 * screen's own Scan Sender Response
 *
 * A person can show the request, leave the receive screen, and scan the sender's response later
 * from the wallet. That scan goes through `ScanManager`, which has to recognize the response, find
 * the stored receive session and reopen the receive screen at the password. The hardware signer
 * host plays the sending phone with the `keyteleport` crate, and the debug QR seam stands in for
 * the camera (docs/testing.md).
 *
 * Runs in the uiTest flavour only, like the other full-launch tests. Needs the fake node and the
 * hardware signer host up and forwarded, which `just android-instrumented-ci` does
 */
@LargeTest
@RunWith(AndroidJUnit4::class)
class KeyTeleportAppScannerFullLaunchTest {
    private companion object {
        /** A BIP39 test vector, a different wallet from the one onboarding imports */
        const val SENT_WORDS = "legal winner thank year wave sausage worth useful legal winner thank yellow"

        // enough frames to scan some, repeat one, and still have more to go
        const val RESPONSE_PARTS = 4
    }

    @get:Rule
    val stayAwake = AndroidDeviceStayAwakeRule()

    private lateinit var device: UiDevice
    private lateinit var qr: QrScript
    private val sender =
        HostControlHttp("127.0.0.1", 39_010) {
            "the hardware signer host is unreachable; run the test through `just android-instrumented-ci`"
        }
    private var previousShowImeWithHardKeyboard: String? = null

    // set once this test starts a receive, so cleanup never ends a session stored before it ran
    private var startedReceive = false

    @Before
    fun prepareDevice() {
        device = fullLaunchDevice()
        qr = QrScript()

        // see UtxoLockFullLaunchTest: the onboarding robot must never see the soft keyboard
        previousShowImeWithHardKeyboard = device.executeShellCommand("settings get secure show_ime_with_hard_keyboard").trim()
        device.executeShellCommand("settings put secure show_ime_with_hard_keyboard 0")
    }

    @After
    fun restoreDevice() {
        try {
            qr.remove()

            // a leftover session makes every later receiving test skip; resuming it in a peer and
            // closing that peer ends it
            if (startedReceive && hasStoredReceiveSession()) {
                KeyTeleportTestPeer().use { peer -> peer.startReceive() }
            }
        } finally {
            when (val previous = previousShowImeWithHardKeyboard) {
                null, "null" -> device.executeShellCommand("settings delete secure show_ime_with_hard_keyboard")
                else -> device.executeShellCommand("settings put secure show_ime_with_hard_keyboard $previous")
            }
        }
    }

    @Test
    fun multiPartResponseScannedFromTheWalletShowsProgressThenReopensTheReceiveAtThePassword() {
        sender.request("GET", "/health")
        launchWithWallet()
        assumeTrue("a KeyTeleport receive session is already stored on this device", !hasStoredReceiveSession())

        val app = AppManager.getInstance()
        startedReceive = true
        onMain { app.pushRoute(RouteFactory().keyTeleportReceive()) }
        val request = awaitRequest(awaitOnMain("the receive screen's manager") { app.getKeyTeleportManager() })
        val (frames, password) = send(request)
        assertTrue("the response must be a BBQr of at least 3 parts", frames.size >= 3)

        // leaving the receive screen keeps its request, so a later scan can still answer it
        leaveReceiveScreen(app)

        // the camera can catch an animated QR at any frame, so two arrive out of order
        retryingStale { device.waitFor(By.desc("QR Code"), timeoutMs = 20_000L).click() }
        qr.show(listOf(frames[2], frames[0]))
        waitForScanProgress("Scanned 2 of ${frames.size}")

        // a frame the camera sees again doesn't count twice
        qr.show(listOf(frames[0]))
        qr.waitUntilScanned(frameCount = 1)
        waitForScanProgress("Scanned 2 of ${frames.size}")
        assertFalse("the scan finished before every part arrived", device.hasObject(By.text("Enter sender password")))

        val rest = frames.filterIndexed { index, _ -> index != 0 && index != 2 }
        qr.show(rest.reversed())
        device.waitFor(By.text("Enter sender password"), timeoutMs = 30_000L)
        untilOnMain("the scan should reopen the receive screen") {
            app.router.routes.lastOrNull() == RouteFactory().keyTeleportReceive()
        }

        retryingStale {
            val field =
                device.wait(Until.findObject(By.clazz("android.widget.EditText")), 10_000L)
                    ?: error("no password field on screen\n${device.hierarchy()}")

            field.text = password
        }

        retryingStale { device.waitFor(By.text("Continue")).click() }
        device.waitFor(By.text("Recovery words received"), timeoutMs = 30_000L)
    }

    private fun launchWithWallet() {
        launchFullApp()
        val wallet = FullLaunchWalletRobot(device)
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
        device.waitFor(By.res("selectedWallet.balance"), timeoutMs = 60_000L)

        // finishing onboarding resets the route a moment later, which would replace a route opened before it
        untilOnMain("the app should open the imported wallet") {
            val router = AppManager.getInstance().router
            router.default is Route.SelectedWallet && router.routes.isEmpty()
        }
    }

    /** Has the host answer the request with [SENT_WORDS], as the sending phone; returns the frames and password */
    private fun send(request: KeyTeleportRequestCopy): Pair<List<String>, String> {
        val body =
            JSONObject()
                .put("request", request.packetText)
                .put("code", request.numericCode)
                .put("payload", JSONObject().put("mnemonic", SENT_WORDS))
                .put("min_parts", RESPONSE_PARTS)

        val response = JSONObject(sender.request("POST", "/keyteleport/send", body))
        val parts: JSONArray = response.getJSONArray("parts")

        return (0 until parts.length()).map(parts::getString) to response.getString("password")
    }

    // on a slow emulator the receive screen can still be recomposing after its request appears,
    // and Back went stale on all three quick tries, so this taps whichever Back is current until
    // the route is gone, for up to 20 s
    private fun leaveReceiveScreen(app: AppManager) {
        val deadline = SystemClock.uptimeMillis() + 20_000L

        while (onReceiveScreen(app)) {
            if (SystemClock.uptimeMillis() > deadline) error("Back should leave the receive screen\n${device.hierarchy()}")

            val back = device.wait(Until.findObject(By.desc("Back")), 2_000L) ?: continue

            try {
                back.click()
            } catch (_: StaleObjectException) {
                continue
            }

            // a tap that landed pops the route, so wait for that before looking for Back again
            val settled = SystemClock.uptimeMillis() + 3_000L
            while (onReceiveScreen(app) && SystemClock.uptimeMillis() < settled) Thread.sleep(100)
        }
    }

    private fun onReceiveScreen(app: AppManager): Boolean {
        val on = AtomicBoolean()
        onMain { on.set(app.router.routes.any { it is Route.KeyTeleport }) }

        return on.get()
    }

    // compose can replace a node between its lookup and the click or edit that follows, which
    // throws before any input is sent, so the block looks its node up again and retries
    private inline fun retryingStale(block: () -> Unit) {
        repeat(2) {
            try {
                return block()
            } catch (_: StaleObjectException) {
                device.waitForIdle()
            }
        }

        block()
    }

    // see HardwareSigningFullLaunchTest: the scanner's progress text updates under a cached tree
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

    private fun untilOnMain(
        what: String,
        condition: () -> Boolean,
    ) {
        awaitOnMain(what) { if (condition()) Unit else null }
    }

    private fun onMain(block: () -> Unit) {
        InstrumentationRegistry.getInstrumentation().runOnMainSync(block)
    }
}
