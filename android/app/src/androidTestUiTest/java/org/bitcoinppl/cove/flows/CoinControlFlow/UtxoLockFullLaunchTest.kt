package org.bitcoinppl.cove.flows.CoinControlFlow

import android.graphics.Rect
import android.os.Process
import android.system.Os
import android.view.WindowInsets
import android.view.accessibility.AccessibilityNodeInfo
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.filters.LargeTest
import androidx.test.platform.app.InstrumentationRegistry
import androidx.test.runner.lifecycle.ActivityLifecycleMonitorRegistry
import androidx.test.runner.lifecycle.Stage
import androidx.test.uiautomator.By
import androidx.test.uiautomator.BySelector
import androidx.test.uiautomator.StaleObjectException
import androidx.test.uiautomator.UiDevice
import androidx.test.uiautomator.UiObject2
import androidx.test.uiautomator.Until
import kotlinx.coroutines.runBlocking
import org.bitcoinppl.cove.AppManager
import org.bitcoinppl.cove.initialScanComplete
import org.bitcoinppl.cove.test.AndroidDeviceStayAwakeRule
import org.bitcoinppl.cove.test.FAKE_NODE_ESPLORA_URL
import org.bitcoinppl.cove.test.FakeNodeBroadcast
import org.bitcoinppl.cove.test.FakeNodeControl
import org.bitcoinppl.cove.test.FakeNodeOutPoint
import org.bitcoinppl.cove.test.FullLaunchOnboardingRobot
import org.bitcoinppl.cove.test.FullLaunchStartupRobot
import org.bitcoinppl.cove.test.fullLaunchDevice
import org.bitcoinppl.cove.test.launchFullApp
import org.bitcoinppl.cove_core.AppAction
import org.bitcoinppl.cove_core.Database
import org.bitcoinppl.cove_core.NodeSelector
import org.bitcoinppl.cove_core.types.Network
import org.json.JSONObject
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.FixMethodOrder
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.junit.runners.MethodSorters
import java.io.ByteArrayOutputStream
import java.io.File
import java.math.BigDecimal
import java.util.regex.Pattern

/**
 * Locks and unlocks UTXOs in the real app against cove-fake-node, then checks that a locked coin
 * stays out of coin control selection and out of a broadcast send
 *
 * The scenario runs as two ordered stages so the locks are checked after a cold relaunch:
 * the test orchestrator gives every test method its own instrumentation run, and starting an
 * instrumentation run force-stops the app, so stage two starts in a fresh process. Stage one
 * records its process id and the funded outpoints in a marker file that stage two consumes, so
 * stage two fails rather than passes on stale state when run alone or in the same process.
 *
 * Drives the app with UiAutomator rather than Compose test APIs: the app bootstraps inside a
 * composition effect, and a Compose test environment runs every composition's effects on its test
 * dispatcher, which stalls a cold launch at bootstrap.
 *
 * Runs in the uiTest flavour only: its connectivity is pinned to connected and its cloud backup
 * providers are scripted, so the scenario doesn't depend on the device's network or Google account.
 * Needs the node up and forwarded, which `just android-instrumented-ci` does
 */
@LargeTest
@RunWith(AndroidJUnit4::class)
@FixMethodOrder(MethodSorters.NAME_ASCENDING)
class UtxoLockFullLaunchTest {
    private companion object {
        val MNEMONIC = List(11) { "abandon" }.plus("about").joinToString(" ")

        // BIP173 test vector: the P2WPKH address of the secp256k1 generator point's public key,
        // which no account of the abandon ×11 about mnemonic derives
        const val EXTERNAL_ADDRESS = "tb1qw508d6qejxtdg4y5r3zarvary0c5xw7kxpjzsx"

        const val SATS_A = 100_000L
        const val SATS_B = 60_000L
        const val SATS_C = 30_000L

        const val MARKER_FILE = "utxo-lock-full-launch-stage.json"
        const val MARKER_MAX_AGE_MS = 30 * 60 * 1_000L
        const val LOCKED_SELECTION_MESSAGE = "Unlock this UTXO before selecting it."
        const val LOCK_TOGGLE_TAG = "transactionDetails.lockToggle"
    }

    private data class Coins(
        val a: FakeNodeOutPoint,
        val b: FakeNodeOutPoint,
        val c: FakeNodeOutPoint,
    )

    @get:Rule
    val stayAwake = AndroidDeviceStayAwakeRule()

    private lateinit var device: UiDevice
    private val node = FakeNodeControl()
    private var previousShowImeWithHardKeyboard: String? = null

    private val markerFile: File
        get() = File(InstrumentationRegistry.getInstrumentation().targetContext.noBackupFilesDir, MARKER_FILE)

    @Before
    fun prepareDevice() {
        device = fullLaunchDevice()
        previousShowImeWithHardKeyboard = device.executeShellCommand("settings get secure show_ime_with_hard_keyboard").trim()

        // the onboarding robot presses Back while the soft keyboard is showing, and if the keyboard
        // is already closing that Back leaves the import screen. With it hidden behind the hardware
        // keyboard until the Max step, the robot never presses Back
        device.executeShellCommand("settings put secure show_ime_with_hard_keyboard 0")

        // debug builds read this on every fee fetch, so sends price fees from the fake node rather
        // than mainnet mempool.space. Each stage is a new process, so it's set again after the relaunch
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
    fun stage1LocksFromCoinControlAndTransactionDetails() {
        markerFile.delete()
        val coins = fundWallet()

        // 1. reset app data, launch, and point the app at Signet and the fake node
        launchFullApp()
        FullLaunchStartupRobot(device).assertBootstrappedIntoOnboarding()
        selectSignetFakeNode()

        // 2. import the funded mnemonic and wait for the scan
        FullLaunchOnboardingRobot(device)
            .tapGetStarted()
            .chooseExistingUser()
            .useAnotherWallet()
            .chooseSoftwareWallet()
            .importKnownEmptyMainnetWalletWords()
            .skipCloudBackupAfterImport()
            .acceptTermsAfterImport()
        waitForBalance(SATS_A + SATS_B + SATS_C)

        // 3. lock B from coin control
        openManageUtxos()
        listOf(coins.a, coins.b, coins.c).forEach { assertLockState(it, locked = false) }
        setLockFromCoinControl(coins.b, lock = true)
        assertLockState(coins.b, locked = true)

        tapLockedRowUntilSnackbar(coins.b)
        assertNothingSelected()

        selectAll()
        assertSelection(sats = SATS_A + SATS_C, count = 2)
        device.pressBack()

        // 4. lock C from its receive transaction's details
        openReceiveTransaction(SATS_C)
        toggleTransactionLock(expectedAfter = "Unlock")
        device.pressBack()

        openManageUtxos()
        assertLockState(coins.c, locked = true)
        assertLockState(coins.b, locked = true)
        selectAll()
        assertSelection(sats = SATS_A, count = 1)
        device.pressBack()

        writeMarker(coins)
    }

    @Test
    fun stage2LocksSurviveRelaunchAndExcludeCoinsFromSends() {
        val coins = consumeMarker()

        // 5. cold relaunch without resetting data; the locks persisted
        launchFullApp(resetData = false)
        waitForBalance(SATS_A + SATS_B + SATS_C)
        openManageUtxos()
        assertLockState(coins.a, locked = false)
        assertLockState(coins.b, locked = true)
        assertLockState(coins.c, locked = true)
        device.pressBack()

        // 6. a Max send spends only the unlocked coin
        sendMaxToExternalAddress()

        val maxSend = node.broadcasts().single()
        assertEquals("the Max send must spend only A", listOf(coins.a), maxSend.inputs)
        assertSendsOnlyToExternalAddress(maxSend)
        assertTrue("B must stay unspent while locked", !node.isSpent(coins.b))
        assertTrue("C must stay unspent while locked", !node.isSpent(coins.c))

        node.mine()
        waitForBalance(SATS_B + SATS_C)

        // 7. unlock B from coin control and C from its transaction details
        openManageUtxos()
        setLockFromCoinControl(coins.b, lock = false)
        assertLockState(coins.b, locked = false)
        device.pressBack()

        openReceiveTransaction(SATS_C)
        toggleTransactionLock(expectedAfter = "Lock")
        device.pressBack()

        openManageUtxos()
        assertLockState(coins.b, locked = false)
        assertLockState(coins.c, locked = false)
        selectAll()
        assertSelection(sats = SATS_B + SATS_C, count = 2)

        // 8. a coin control send spends the unlocked coins
        device.waitFor(By.res("coinControl.continue")).click()
        enterExternalAddress()
        clickNext()
        swipeToSend()

        val broadcasts = node.broadcasts()
        assertEquals("expected the Max send and the coin control send", 2, broadcasts.size)
        assertEquals(
            "the coin control send must spend exactly B and C",
            setOf(coins.b, coins.c),
            broadcasts[1].inputs.toSet(),
        )
        assertEquals(2, broadcasts[1].inputs.size)
    }

    private fun fundWallet(): Coins {
        node.reset()
        val outputs = node.fundWallet(MNEMONIC, listOf(0 to SATS_A, 1 to SATS_B, 2 to SATS_C))
        node.mine()

        fun outpointFor(sats: Long) = outputs.single { it.sats == sats }.outpoint

        return Coins(a = outpointFor(SATS_A), b = outpointFor(SATS_B), c = outpointFor(SATS_C))
    }

    // onboarding has no route to network or node settings, so this does in process what
    // Settings → Network and Settings → Node do
    private fun selectSignetFakeNode() {
        val instrumentation = InstrumentationRegistry.getInstrumentation()
        var dispatched = false
        instrumentation.runOnMainSync {
            dispatched = AppManager.getInstance().dispatch(AppAction.ChangeNetwork(Network.SIGNET))
        }
        assertTrue("expected the network change to dispatch", dispatched)

        runBlocking {
            NodeSelector().use { selector ->
                val node = selector.parseCustomNode(FAKE_NODE_ESPLORA_URL, "Custom Esplora", "Fake node")
                selector.checkAndSaveNode(node)
            }
        }

        Database().globalConfig().use { config ->
            assertEquals(Network.SIGNET, config.selectedNetwork())
            assertEquals(FAKE_NODE_ESPLORA_URL, config.selectedNode().url.trimEnd('/'))
        }
    }

    // coin control and send refuse to open until the initial full scan completes, which can finish
    // after the balance first shows
    private fun waitForBalance(sats: Long) {
        val deadline = System.currentTimeMillis() + 90_000L
        var lastText: String? = null

        while (System.currentTimeMillis() < deadline) {
            dismissWalletSelectionSheetIfShown()

            // the balance renders only after bootstrap, before which AppManager can't be read
            lastText = device.findObject(By.res("selectedWallet.balance"))?.anyText()
            if (lastText?.let(::parseSats) == sats && isInitialScanComplete()) return

            Thread.sleep(250)
        }

        error("Timed out waiting for a balance of $sats sats after the initial scan, last shown \"$lastText\"\n${device.hierarchy()}")
    }

    private fun isInitialScanComplete(): Boolean {
        var complete = false
        InstrumentationRegistry.getInstrumentation().runOnMainSync {
            complete = AppManager.getInstance().walletManager?.ledgerState?.initialScanComplete == true
        }

        return complete
    }

    // the import can offer other address types when they have history; the fake node gives them none,
    // but keep the native segwit wallet if the sheet shows anyway
    private fun dismissWalletSelectionSheetIfShown() {
        device.findObject(By.text("Keep Current"))?.click()
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

    private fun utxoRow(outpoint: FakeNodeOutPoint): UiObject2 = device.waitFor(By.res(utxoTag(outpoint)))

    private fun utxoTag(outpoint: FakeNodeOutPoint): String = "coinControl.utxo.$outpoint"

    private fun setLockFromCoinControl(
        outpoint: FakeNodeOutPoint,
        lock: Boolean,
    ) {
        utxoRow(outpoint).longClick()
        device.waitFor(By.text(if (lock) "Lock UTXO" else "Unlock UTXO")).click()
        device.wait(Until.gone(By.text(if (lock) "Lock UTXO" else "Unlock UTXO")), 5_000L)
    }

    // a tap right after the lock menu closes can be swallowed by its dismissal; retrying is safe
    // because the row is locked and a tap can never select it
    private fun tapLockedRowUntilSnackbar(outpoint: FakeNodeOutPoint) {
        repeat(3) {
            device.waitForIdle()
            utxoRow(outpoint).click()
            if (device.wait(Until.hasObject(By.text(LOCKED_SELECTION_MESSAGE)), 3_000L)) {
                // the snackbar covers Continue, which the next checks read
                device.wait(Until.gone(By.text(LOCKED_SELECTION_MESSAGE)), 15_000L)
                return
            }
        }

        error("tapping locked $outpoint never showed \"$LOCKED_SELECTION_MESSAGE\"\n${device.hierarchy()}")
    }

    // uiautomator doesn't expose stateDescription, so read it from the accessibility tree; iOS
    // exposes the same strings as the row's accessibility value
    private fun assertLockState(
        outpoint: FakeNodeOutPoint,
        locked: Boolean,
    ) {
        val tag = utxoTag(outpoint)
        val expected = if (locked) "locked" else "unlocked"
        val deadline = System.currentTimeMillis() + 10_000L
        var actual: String? = null

        while (System.currentTimeMillis() < deadline) {
            actual = stateDescription(tag)
            if (actual == expected) return

            Thread.sleep(100)
        }

        error("expected $tag to be $expected, was $actual\n${device.hierarchy()}")
    }

    private fun stateDescription(resourceName: String): String? {
        val uiAutomation = InstrumentationRegistry.getInstrumentation().uiAutomation

        return uiAutomation.windows
            .mapNotNull { it.root }
            .firstNotNullOfOrNull { it.findByResourceName(resourceName) }
            ?.stateDescription
            ?.toString()
    }

    private fun AccessibilityNodeInfo.findByResourceName(resourceName: String): AccessibilityNodeInfo? {
        if (viewIdResourceName == resourceName) return this

        return (0 until childCount)
            .asSequence()
            .mapNotNull { getChild(it) }
            .firstNotNullOfOrNull { it.findByResourceName(resourceName) }
    }

    private fun selectAll() {
        device.waitFor(By.res("coinControl.selectAll").text("Select All")).click()
        device.waitFor(By.res("coinControl.selectAll").text("Deselect All"))
    }

    private fun assertNothingSelected() {
        assertNull("a locked UTXO must not be selectable", device.findObject(By.res("coinControl.selectedTotal")))
        assertEquals("Continue", device.waitFor(By.res("coinControl.continue")).anyText())
        assertEquals("Select All", device.waitFor(By.res("coinControl.selectAll")).anyText())
    }

    private fun assertSelection(
        sats: Long,
        count: Int,
    ) {
        val total = device.waitFor(By.res("coinControl.selectedTotal")).anyText().orEmpty()
        assertEquals("selected total \"$total\"", sats, parseSats(total))
        assertEquals("Continue ($count)", device.waitFor(By.res("coinControl.continue")).anyText())
    }

    private fun openReceiveTransaction(sats: Long) {
        // rows show "0.0003 BTC" or "30,000 SATS" depending on the wallet unit, so match by value
        val amounts = By.text(Pattern.compile("(?i).*\\d.*(BTC|SATS)"))
        val deadline = System.currentTimeMillis() + 20_000L

        while (System.currentTimeMillis() < deadline) {
            device.waitForIdle()

            // the list recomposes as wallet updates land, which can invalidate a node mid scan
            val row =
                try {
                    device.findObjects(amounts).firstOrNull { node ->
                        parseSats(node.text) == sats && device.isTappable(node)
                    }
                } catch (_: StaleObjectException) {
                    continue
                }

            if (row != null) {
                row.click()
                expandTransactionDetails()
                return
            }

            device.swipe(device.displayWidth / 2, device.displayHeight * 2 / 3, device.displayWidth / 2, device.displayHeight / 3, 20)
            Thread.sleep(250)
        }

        error("no transaction row for $sats sats\n${device.hierarchy()}")
    }

    // expansion is saved in the wallet metadata, so a later visit may already show the details
    private fun expandTransactionDetails() {
        val deadline = System.currentTimeMillis() + 20_000L

        while (System.currentTimeMillis() < deadline) {
            device.waitForIdle()
            if (device.hasObject(By.res(LOCK_TOGGLE_TAG))) return

            // the toggle sits at the bottom of the details and the expanded rows push it below the fold
            val toggle = device.findObject(By.res("transactionDetails.showDetails"))
            when {
                toggle == null -> device.swipe(device.displayWidth / 2, device.displayHeight * 2 / 3, device.displayWidth / 2, device.displayHeight / 3, 20)

                toggle.anyText() == "Show Details" -> {
                    toggle.click()
                    device.wait(Until.hasObject(By.res(LOCK_TOGGLE_TAG)), 5_000L)
                }

                else -> Thread.sleep(250)
            }
        }

        error("transaction details didn't expand\n${device.hierarchy()}")
    }

    // retrying is safe because it only taps while the label still reads the starting state
    private fun toggleTransactionLock(expectedAfter: String) {
        val before = if (expectedAfter == "Unlock") "Lock" else "Unlock"
        val deadline = System.currentTimeMillis() + 20_000L

        while (System.currentTimeMillis() < deadline) {
            // the label is briefly replaced by a progress indicator while the lock change saves
            val toggle = device.scrollTo(By.res(LOCK_TOGGLE_TAG))
            when (toggle.anyText()) {
                expectedAfter -> return

                before -> {
                    toggle.click()

                    // BySelector builders mutate in place, so the wait gets its own selector
                    device.wait(Until.hasObject(By.res(LOCK_TOGGLE_TAG).hasDescendant(By.text(expectedAfter))), 5_000L)
                }

                else -> Thread.sleep(250)
            }
        }

        error("expected the transaction lock toggle to read $expectedAfter\n${device.hierarchy()}")
    }

    private fun sendMaxToExternalAddress() {
        // Max lives in a toolbar shown only while the soft keyboard is up, which an emulator with a
        // hardware keyboard hides unless this is set. It stays off until here because the onboarding
        // robot dismisses a visible keyboard with Back, which can race the import screen
        device.executeShellCommand("settings put secure show_ime_with_hard_keyboard 1")

        device.waitFor(By.text("Send")).click()
        enterExternalAddress()

        device.waitFor(By.res("sendFlow.amount")).click()
        device.waitFor(By.res("sendFlow.max")).click()
        waitUntil("expected Max to fill in the amount") {
            device.findObject(By.res("sendFlow.amount"))?.anyText()?.any { it in '1'..'9' } == true
        }

        // the field shows the number alone, with the unit beside it; a BTC amount always has a point here
        val maxText = device.waitFor(By.res("sendFlow.amount")).anyText().orEmpty()
        val maxNumber = BigDecimal(maxText.filter { it.isDigit() || it == '.' })
        val maxSats = if ('.' in maxText) maxNumber.movePointRight(8).longValueExact() else maxNumber.longValueExact()
        assertTrue("Max filled \"$maxText\", more than unlocked A can cover", maxSats in 1 until SATS_A)

        clickNext()
        swipeToSend()
    }

    private fun enterExternalAddress() {
        // the send screen opens with the amount focused, and the keyboard sliding in can move the
        // address field out from under a tap; tapping again is harmless once it has focus
        for (attempt in 1..3) {
            device.waitForIdle()
            device.scrollTo(By.res("sendFlow.address")).click()
            if (device.wait(Until.hasObject(By.res("sendFlow.address").focused(true)), 3_000L)) break
        }

        device.waitFor(By.res("sendFlow.address").focused(true))
        device.findObject(By.res("sendFlow.address")).text = EXTERNAL_ADDRESS

        // once the address is accepted the field can lose focus and show it in groups of four
        waitUntil("expected the address field to hold the external address") {
            try {
                device.findObjects(By.textStartsWith("tb1q")).any { node ->
                    node.text?.filterNot { it.isWhitespace() } == EXTERNAL_ADDRESS
                }
            } catch (_: StaleObjectException) {
                false
            }
        }
    }

    private fun clickNext() {
        dismissKeyboard()
        device.scrollTo(By.res("sendFlow.next")).click()
        device.waitFor(By.res("sendFlowConfirm.swipeToSend"), timeoutMs = 30_000L)
    }

    private fun swipeToSend() {
        val track = device.waitFor(By.res("sendFlowConfirm.swipeToSend")).visibleBounds
        device.swipe(track.left + 40, track.centerY(), track.right - 4, track.centerY(), 40)

        device.waitFor(By.text("Transaction sent successfully!"), timeoutMs = 60_000L)
        device.waitFor(By.text("OK")).click()
        device.waitFor(By.res("selectedWallet.balance"), timeoutMs = 30_000L)
    }

    private fun assertSendsOnlyToExternalAddress(broadcast: FakeNodeBroadcast) {
        assertEquals("a Max send has no change output", 1, broadcast.outputs.size)
        assertEquals(EXTERNAL_ADDRESS, broadcast.outputs.single().address)
    }

    private fun dismissKeyboard() {
        if (!isImeVisible()) return

        device.pressBack()
        waitUntil("expected the keyboard to close") { !isImeVisible() }
    }

    private fun isImeVisible(): Boolean {
        var visible = false
        InstrumentationRegistry.getInstrumentation().runOnMainSync {
            visible =
                ActivityLifecycleMonitorRegistry
                    .getInstance()
                    .getActivitiesInStage(Stage.RESUMED)
                    .firstOrNull()
                    ?.window
                    ?.decorView
                    ?.rootWindowInsets
                    ?.isVisible(WindowInsets.Type.ime()) == true
        }

        return visible
    }

    private fun writeMarker(coins: Coins) {
        val marker =
            JSONObject()
                .put("pid", Process.myPid())
                .put("createdAt", System.currentTimeMillis())
                .put("a", coins.a.toString())
                .put("b", coins.b.toString())
                .put("c", coins.c.toString())

        markerFile.writeText(marker.toString())
    }

    private fun consumeMarker(): Coins {
        assertTrue(
            "stage 2 needs stage 1 of this class to have just passed; run the whole class",
            markerFile.exists(),
        )

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

        return Coins(
            a = FakeNodeOutPoint.parse(marker.getString("a")),
            b = FakeNodeOutPoint.parse(marker.getString("b")),
            c = FakeNodeOutPoint.parse(marker.getString("c")),
        )
    }

    // amounts show as e.g. "130,000 SATS" or "0.0013 BTC" depending on the wallet's unit
    private fun parseSats(text: String): Long? {
        val digits = text.filter { it.isDigit() || it == '.' }
        if (digits.isEmpty()) return null

        return when {
            text.contains("BTC", ignoreCase = true) -> BigDecimal(digits).movePointRight(8).longValueExact()
            text.contains("SAT", ignoreCase = true) -> digits.toLongOrNull()
            else -> null
        }
    }

    private fun waitUntil(
        message: String,
        timeoutMs: Long = 20_000L,
        condition: () -> Boolean,
    ) {
        val deadline = System.currentTimeMillis() + timeoutMs

        while (System.currentTimeMillis() < deadline) {
            if (condition()) return

            Thread.sleep(100)
        }

        assertTrue("$message\n${device.hierarchy()}", condition())
    }

    private fun UiDevice.waitFor(
        selector: BySelector,
        timeoutMs: Long = 20_000L,
    ): UiObject2 = wait(Until.findObject(selector), timeoutMs) ?: error("Timed out waiting for $selector\n${hierarchy()}")

    private fun UiDevice.scrollTo(
        selector: BySelector,
        timeoutMs: Long = 20_000L,
    ): UiObject2 {
        val deadline = System.currentTimeMillis() + timeoutMs

        while (System.currentTimeMillis() < deadline) {
            findObject(selector)?.let { node ->
                if (isTappable(node)) {
                    settledObject(selector)?.takeIf { isTappable(it) }?.let { return it }
                }
            }

            swipe(displayWidth / 2, displayHeight * 2 / 3, displayWidth / 2, displayHeight / 3, 20)
            Thread.sleep(250)
        }

        error("Timed out scrolling to $selector\n${hierarchy()}")
    }

    // a tap in the 48dp gesture navigation strip goes to the system instead of the app
    private fun UiDevice.isTappable(node: UiObject2): Boolean {
        val density = InstrumentationRegistry.getInstrumentation().targetContext.resources.displayMetrics.density

        return node.visibleBounds.bottom <= displayHeight - (48 * density).toInt()
    }

    // a swipe keeps flinging after it ends, and a tap on a moving node lands on whatever scrolls under it
    private fun UiDevice.settledObject(selector: BySelector): UiObject2? {
        var lastBounds: Rect? = null

        repeat(10) {
            val node = findObject(selector) ?: return null
            val bounds =
                try {
                    node.visibleBounds
                } catch (_: StaleObjectException) {
                    null
                }

            if (bounds != null && bounds == lastBounds) return node

            lastBounds = bounds
            Thread.sleep(200)
        }

        return null
    }

    // a tag on a layout wrapper puts the text on a child node
    private fun UiObject2.anyText(): String? = text ?: children.firstNotNullOfOrNull { it.anyText() }

    private fun UiDevice.hierarchy(): String =
        ByteArrayOutputStream().use { output ->
            dumpWindowHierarchy(output)
            output.toString(Charsets.UTF_8.name()).take(120_000)
        }
}
