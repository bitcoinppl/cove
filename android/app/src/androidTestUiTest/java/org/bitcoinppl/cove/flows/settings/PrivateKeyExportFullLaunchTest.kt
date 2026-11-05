package org.bitcoinppl.cove.flows.settings

import android.content.ClipData
import android.content.ClipboardManager
import android.content.Context
import android.graphics.Rect
import android.os.SystemClock
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.filters.LargeTest
import androidx.test.platform.app.InstrumentationRegistry
import androidx.test.uiautomator.By
import androidx.test.uiautomator.BySelector
import androidx.test.uiautomator.StaleObjectException
import androidx.test.uiautomator.UiDevice
import androidx.test.uiautomator.UiObject2
import androidx.test.uiautomator.Until
import kotlinx.coroutines.runBlocking
import org.bitcoinppl.cove.AppManager
import org.bitcoinppl.cove.AuthManager
import org.bitcoinppl.cove.cloudbackup.CloudBackupManager
import org.bitcoinppl.cove.flows.keyteleport.KeyTeleportFlowDirection
import org.bitcoinppl.cove.flows.keyteleport.KeyTeleportResponseCopy
import org.bitcoinppl.cove.flows.keyteleport.KeyTeleportTestPeer
import org.bitcoinppl.cove.flows.keyteleport.awaitRequest
import org.bitcoinppl.cove.flows.keyteleport.awaitResponse
import org.bitcoinppl.cove.flows.keyteleport.hasStoredReceiveSession
import org.bitcoinppl.cove.flows.keyteleport.ingestKeyTeleportText
import org.bitcoinppl.cove.test.FullLaunchOnboardingRobot
import org.bitcoinppl.cove.test.FullLaunchTestRule
import org.bitcoinppl.cove.test.ManualFullLaunchTest
import org.bitcoinppl.cove.test.QrScript
import org.bitcoinppl.cove.test.TestWalletWords
import org.bitcoinppl.cove.test.awaitOnMain
import org.bitcoinppl.cove.test.deleteTestWallet
import org.bitcoinppl.cove.test.dumpWindowHierarchy
import org.bitcoinppl.cove.test.fullLaunchDevice
import org.bitcoinppl.cove.test.importTestWalletOrNull
import org.bitcoinppl.cove.test.launchFullApp
import org.bitcoinppl.cove.test.restoreNoAppLock
import org.bitcoinppl.cove.testconfig.ScriptedCloudBackupFixture
import org.bitcoinppl.cove.testconfig.ScriptedCloudStorageAccess
import org.bitcoinppl.cove.testconfig.ScriptedPasskeyProvider
import org.bitcoinppl.cove_core.AppLockReason
import org.bitcoinppl.cove_core.AuthManagerAction
import org.bitcoinppl.cove_core.CloudBackupRootPrompt
import org.bitcoinppl.cove_core.Database
import org.bitcoinppl.cove_core.KeyTeleportManagerAction
import org.bitcoinppl.cove_core.KeyTeleportManagerState
import org.bitcoinppl.cove_core.Mnemonic
import org.bitcoinppl.cove_core.Route
import org.bitcoinppl.cove_core.RouteFactory
import org.bitcoinppl.cove_core.SettingsRoute
import org.bitcoinppl.cove_core.WalletSettingsRoute
import org.bitcoinppl.cove_core.types.WalletId
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertTrue
import org.junit.Assume.assumeTrue
import org.junit.Before
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

/**
 * Export Private Key, and KeyTeleport, on a wallet that holds only an extended private key
 *
 * The wallet comes from restoring the scripted cloud backup, the same restore a person runs after
 * reinstalling, so no test-only path creates it. The export waits for the real lock screen. A
 * second KeyTeleport manager plays the other phone, in place of the camera between two devices
 */
@ManualFullLaunchTest
@LargeTest
@RunWith(AndroidJUnit4::class)
class PrivateKeyExportFullLaunchTest {
    private lateinit var device: UiDevice

    // leaving decoy mode reloads the wallet list, so the id is kept from the restore
    private lateinit var walletId: WalletId

    // set once this test starts a receive on the app's own manager, so cleanup never ends a session
    // that was stored before the test ran
    private var startedReceive = false

    // the bystander this test imported, deleted afterwards
    private var bystanderId: WalletId? = null

    @get:Rule
    val fullLaunch = FullLaunchTestRule()

    @Before
    fun prepareDevice() {
        ScriptedPasskeyProvider.reset()
        device = fullLaunchDevice()
    }

    @After
    fun cleanUp() {
        try {
            // a leftover session makes every later receiving test skip
            if (startedReceive && hasStoredReceiveSession()) {
                KeyTeleportTestPeer().use { peer -> peer.startReceive() }
            }

            bystanderId?.let(::deleteTestWallet)
        } finally {
            // the next test may host LockView without relaunching, and refuses to run over a leftover PIN
            restoreNoAppLock()
        }
    }

    @Test
    fun bothExportRoutesNeedAFreshMainPinAndCarryTheSameKey() {
        assumeNoStoredReceiveSession()
        restorePrivateKeyWallet()
        setPins()

        // Reveal shows the key after the main PIN, and Copy puts that same key on the clipboard
        openWalletSettings()
        requestExport(REVEAL_ACTION)
        device.enterPin(MAIN_PIN)
        device.waitForText(REVEAL_TITLE)
        device.waitForText(FIXTURE_XPRV)
        clearClipboard()
        device.clickText("Copy")
        waitUntil("Copy should put the private key on the clipboard") { clipboardText() == FIXTURE_XPRV }
        device.clickText("Done")
        assertTrue(device.wait(Until.gone(By.text(REVEAL_TITLE)), 5_000L))

        // a wrong PIN keeps the prompt up and the decoy PIN drops the request, so neither starts a send
        openWalletSettings()
        requestExport(KEY_TELEPORT_ACTION)
        device.enterPin(WRONG_PIN)
        device.assertNoKeyTeleportSendFor(REVEAL_SETTLE_MS)
        device.enterPin(DECOY_PIN)
        waitUntil("decoy PIN should switch to decoy mode") { AuthManager.getInstance().isInDecoyMode() }
        device.assertNoKeyTeleportSendFor(REVEAL_SETTLE_MS)

        onMain { AuthManager.getInstance().lock(AppLockReason.REAUTHENTICATION) }
        device.waitForLockScreen()
        device.enterPin(MAIN_PIN)
        waitUntil("main PIN should leave decoy mode") { !AuthManager.getInstance().isInDecoyMode() }
        waitForAppOnWallet()
        device.assertNoKeyTeleportSendFor(REVEAL_SETTLE_MS)

        // the main PIN starts a send of this wallet, and the other phone receives the revealed key
        openWalletSettings()
        requestExport(KEY_TELEPORT_ACTION)
        device.enterPin(MAIN_PIN)
        device.waitForText(SEND_TITLE)
        val sender = onMainValue { AppManager.getInstance().getKeyTeleportManager() }

        KeyTeleportTestPeer().use { receiver ->
            val request = receiver.startReceive()
            putOnClipboard(request.packetText)
            device.clickText("Paste")
            device.typeIntoField(request.numericCode)
            device.clickText("Continue")

            val response = awaitResponse(sender)
            assertEquals(
                "the other phone received a different key from the one Reveal showed",
                FIXTURE_XPRV,
                receivePrivateKey(receiver, response),
            )
        }

        device.clickText("Done")
        assertTrue(device.wait(Until.gone(By.text(SEND_TITLE)), 5_000L))
    }

    @Test
    fun receivedPrivateKeyStaysHiddenUntilRevealedAndImportsAsAWallet() {
        assumeNoStoredReceiveSession()
        restorePrivateKeyWallet()

        // the private-key wallet is deleted below, and a second wallet keeps the app on a wallet
        val bystander = importTestWalletOrNull(BYSTANDER_WALLET)
        assumeTrue("the bystander test wallet already exists on this device", bystander != null)
        bystanderId = requireNotNull(bystander).id
        setMainPin()

        val app = AppManager.getInstance()
        startedReceive = true
        onMain { app.pushRoute(RouteFactory().keyTeleportReceive()) }
        val request = awaitRequest(onMainValue { app.getKeyTeleportManager() })
        val response = KeyTeleportTestPeer().use { sender -> sender.sendWallet(walletId, request) }

        // without the wallet on this phone, Import Wallet has to create it from the received key;
        // deleting the selected wallet replaces the route, which closes the screen's manager, so
        // reopening the screen resumes the stored session in a new one, read through receiveState
        deleteTestWallet(walletId)
        waitUntil("the private-key wallet should be deleted") { app.wallets.none { it.id == walletId } }
        dismissCloudBackupVerificationPrompts()
        onMain {
            if (app.router.routes.lastOrNull() != RouteFactory().keyTeleportReceive()) {
                app.pushRoute(RouteFactory().keyTeleportReceive())
            }
        }

        scanSenderResponse(response)
        awaitOnMain("the receiver to ask for the password") {
            receiveState().takeIf { it is KeyTeleportManagerState.ReceiveEnterPassword }
        }
        device.typeIntoField(response.password)
        device.clickText("Continue")
        device.waitForText(XPRV_REVIEW_TITLE)

        // the key shows only after Reveal, and Hide conceals it again
        device.assertPrivateKeyNotShown()
        device.clickText("Reveal")
        device.waitForText(FIXTURE_XPRV)
        device.clickText("Hide")
        assertTrue("Hide left the private key on screen", device.wait(Until.gone(By.text(FIXTURE_XPRV)), 5_000L))
        device.waitForText("Reveal")

        device.clickText("Reveal")
        device.waitForText(FIXTURE_XPRV)
        clearClipboard()
        device.clickText("Copy")
        waitUntil("Copy should put the received key on the clipboard") { clipboardText() == FIXTURE_XPRV }

        // leaving the app with the key revealed shows the PIN cover first, and the key stays hidden after
        device.pressHome()
        waitUntil("the app lock should engage in the background") { AuthManager.getInstance().isLocked }
        Thread.sleep(GRACE_PERIOD_MS)
        launchFullApp(resetData = false)
        device.waitForLockScreen()
        device.assertPrivateKeyNotShown()
        device.enterPin(MAIN_PIN)
        dismissCloudBackupVerificationPrompts()
        device.waitForText(XPRV_REVIEW_TITLE)
        device.waitForText("Reveal")
        device.assertPrivateKeyNotShown()

        device.clickText("Import Wallet")
        device.waitForText("Wallet imported")
        dismissCloudBackupVerificationPrompts()
        val imported =
            awaitOnMain("the received key to import") {
                (receiveState() as? KeyTeleportManagerState.ReceiveImportedWallet)?.v1?.id
            }

        device.clickText("Done")
        waitUntil("Done should open the imported wallet") {
            app.router.default == Route.SelectedWallet(imported) && app.router.routes.isEmpty()
        }
        assertEquals("the imported wallet holds a different key", FIXTURE_XPRV, readPrivateKey(imported))
        assertEquals("the imported key was added more than once", 1, walletsHoldingTheFixtureKey())
    }

    // the private-key counterpart of RepeatedSecretAccessTest, which covers recovery words
    @Test
    fun repeatedPrivateKeyReadsWritesAndSendsKeepOneReadableWallet() {
        assumeNoStoredReceiveSession()
        restorePrivateKeyWallet()

        // a mnemonic wallet stands in for the user's other wallets, whose secrets must not change
        val bystander = importTestWalletOrNull(BYSTANDER_WALLET)
        assumeTrue("the bystander test wallet already exists on this device", bystander != null)
        val bystanderId = requireNotNull(bystander).id
        this.bystanderId = bystanderId
        val otherWallets = walletIds() - walletId

        repeat(READ_ROUNDS) { round ->
            assertEquals("read $round returned a different key", FIXTURE_XPRV, readPrivateKey(walletId))
        }

        // importing a key the phone already holds writes nothing, so each write deletes the wallet first
        repeat(WRITE_ROUNDS) { round ->
            val result =
                sendToThisDevice { receiver, sent ->
                    deleteTestWallet(walletId)
                    waitUntil("write $round should delete the wallet") { walletId !in walletIds() }
                    importReceivedKey(receiver, sent, round)
                }

            val imported = result as? KeyTeleportManagerState.ReceiveImportedWallet
            assertNotNull("write $round found the deleted wallet still listed", imported)
            walletId = requireNotNull(imported).v1.id
            assertEquals("write $round read back a different key", FIXTURE_XPRV, readPrivateKey(walletId))
        }

        repeat(SEND_ROUNDS) { round ->
            val result = sendToThisDevice { receiver, sent -> importReceivedKey(receiver, sent, round) }
            val existing = (result as? KeyTeleportManagerState.ReceiveAlreadyImportedWallet)?.v1?.id
            assertEquals("send $round did not find the existing wallet", walletId, existing)
        }

        assertEquals("the private-key wallet is not listed exactly once", 1, walletIds().count { it == walletId })
        assertEquals("the other wallets changed", otherWallets, walletIds() - walletId)
        assertEquals("the key changed after the sends", FIXTURE_XPRV, readPrivateKey(walletId))
        assertEquals("the bystander's recovery words changed", BYSTANDER_WORDS, Mnemonic(id = bystanderId).use { it.words() })
    }

    @Test
    fun onlyAFreshMainPinRevealsThePrivateKey() {
        restorePrivateKeyWallet()
        setPins()

        // leaving the app at the prompt cancels the request, so a later unlock reveals nothing
        openWalletSettings()
        requestReveal()
        device.pressHome()
        launchFullApp(resetData = false)
        device.waitForLockScreen()
        device.enterPin(MAIN_PIN)
        device.assertPrivateKeyHiddenFor(REVEAL_SETTLE_MS)

        // the decoy PIN opens the decoy wallets and drops the request
        openWalletSettings()
        requestReveal()
        device.enterPin(DECOY_PIN)
        waitUntil("decoy PIN should switch to decoy mode") { AuthManager.getInstance().isInDecoyMode() }
        device.assertPrivateKeyHiddenFor(REVEAL_SETTLE_MS)

        onMain { AuthManager.getInstance().lock(AppLockReason.REAUTHENTICATION) }
        device.waitForLockScreen()
        device.enterPin(MAIN_PIN)
        waitUntil("main PIN should leave decoy mode") { !AuthManager.getInstance().isInDecoyMode() }
        waitForAppOnWallet()
        device.assertPrivateKeyHiddenFor(REVEAL_SETTLE_MS)

        // a wrong PIN keeps the prompt up, then the main PIN entered after the request reveals
        openWalletSettings()
        requestReveal()
        device.enterPin(WRONG_PIN)
        device.assertPrivateKeyHiddenFor(REVEAL_SETTLE_MS)
        assertNotNull("a wrong PIN should keep the lock screen up", device.findObject(By.text(LOCK_TITLE)))

        device.enterPin(MAIN_PIN)
        device.waitForText(REVEAL_TITLE)
        device.waitForText(FIXTURE_XPRV)
        device.clickText("Done")
        assertTrue(device.wait(Until.gone(By.text(REVEAL_TITLE)), 5_000L))
    }

    @Test
    fun exportWithoutAppPinIsRefused() {
        restorePrivateKeyWallet()
        assertFalse(AuthManager.getInstance().isAuthEnabled)

        openWalletSettings()
        device.clickClickableAncestor(device.scrollUntilText(EXPORT_ITEM))
        device.clickText("Continue")
        device.clickText("Reveal")

        device.waitForText("Private Key Export")
        device.waitForText("Set up a PIN or biometric unlock before exporting a private key.")
        device.clickText("OK")

        assertTrue(device.findObject(By.text(LOCK_TITLE)) == null)
        device.assertPrivateKeyHiddenFor(REVEAL_SETTLE_MS)
    }

    // receiving deletes the stored receive session afterwards, so never run over someone's pending transfer
    private fun assumeNoStoredReceiveSession() {
        assumeTrue("a KeyTeleport receive session is already stored on this device", !hasStoredReceiveSession())
    }

    private fun restorePrivateKeyWallet() {
        ScriptedCloudStorageAccess.configurePrivateKeyWalletRestore()
        launchFullApp()

        device.clickClickableAncestor(device.waitForText("Restore with Passkey"))
        device.waitForText("You're all set")
        device.clickClickableAncestor(device.waitForText("Done"))
        FullLaunchOnboardingRobot(device).acceptTermsAfterImport()
        waitUntil("the private-key wallet should be restored") { walletIdOrNull() != null }
        walletId = requireNotNull(walletIdOrNull())
        waitForAppOnWallet()
    }

    // finishing onboarding and leaving decoy mode each reset the app's route a moment later, which
    // would replace any route the test opened before it
    private fun waitForAppOnWallet() {
        waitUntil("the app should open the private-key wallet") {
            val router = AppManager.getInstance().router
            router.default == Route.SelectedWallet(walletId) && router.routes.isEmpty()
        }
    }

    private fun setPins() {
        setMainPin()
        val auth = AuthManager.getInstance()
        onMain { auth.setDecoyPin(DECOY_PIN) }
        waitUntil("the decoy PIN should be saved") { auth.isDecoyPinEnabled }
    }

    private fun setMainPin() {
        val auth = AuthManager.getInstance()
        onMain { auth.dispatch(AuthManagerAction.SetPin(MAIN_PIN)) }
        waitUntil("the main PIN should enable the app lock") { auth.isAuthEnabled }
    }

    private fun openWalletSettings() {
        onMain { AppManager.getInstance().resetRoute(Route.Settings(SettingsRoute.Main)) }
        device.waitForText("Security")

        // pushed onto Settings, the same stack tapping the wallet's row builds
        onMain {
            AppManager.getInstance().pushRoute(
                Route.Settings(SettingsRoute.Wallet(id = walletId, route = WalletSettingsRoute.MAIN)),
            )
        }
        // the wallet's name is also on the Settings list, so wait for a section only its own settings have
        device.waitForText(WALLET_SETTINGS_SECTION)
    }

    private fun requestReveal() {
        requestExport(REVEAL_ACTION)
    }

    private fun requestExport(action: String) {
        device.clickClickableAncestor(device.scrollUntilText(EXPORT_ITEM))
        device.waitForText("Are you sure?")
        device.clickText("Continue")
        device.waitForText("Export Private Key")
        device.clickText(action)
        device.waitForLockScreen()
    }

    private fun scanSenderResponse(response: KeyTeleportResponseCopy) {
        val qr = QrScript()
        try {
            device.clickText("Scan Sender Response")
            qr.show(listOf(response.packetText))
            qr.waitUntilScanned(frameCount = 1)
        } finally {
            qr.remove()
        }
    }

    /** Receive [response] on [receiver] and return the key it carries, without importing it */
    private fun receivePrivateKey(
        receiver: KeyTeleportTestPeer,
        response: KeyTeleportResponseCopy,
    ): String? {
        unlockReceivedKey(receiver, response, "the send")
        return awaitOnMain("the received key to reveal") { receiver.manager.secrets.revealXprv() }
    }

    private fun unlockReceivedKey(
        receiver: KeyTeleportTestPeer,
        response: KeyTeleportResponseCopy,
        what: String,
    ) {
        val accepted =
            awaitOnMain("$what to reach the receiver") {
                receiver.manager.ingestKeyTeleportText(response.packetText, KeyTeleportFlowDirection.RECEIVE)
            }

        assertTrue("$what response was rejected", accepted)
        awaitOnMain("$what to ask for the password") {
            receiver.manager.state.takeIf { it is KeyTeleportManagerState.ReceiveEnterPassword }
        }

        receiver.manager.dispatch(KeyTeleportManagerAction.EnterSenderPassword(response.password))
        awaitOnMain("$what to unlock as a private key") {
            receiver.manager.state.takeIf { it is KeyTeleportManagerState.ReceiveXprvReview }
        }
    }

    /**
     * Send the private-key wallet from one test manager to another on this device, run [beforeImport]
     * once the receiver holds the key, and return the receiver's state after importing it
     */
    private fun sendToThisDevice(
        beforeImport: (KeyTeleportTestPeer, KeyTeleportResponseCopy) -> KeyTeleportManagerState,
    ): KeyTeleportManagerState =
        KeyTeleportTestPeer().use { receiver ->
            val request = receiver.startReceive()
            val response = KeyTeleportTestPeer().use { sender -> sender.sendWallet(walletId, request) }
            beforeImport(receiver, response)
        }

    private fun importReceivedKey(
        receiver: KeyTeleportTestPeer,
        response: KeyTeleportResponseCopy,
        round: Int,
    ): KeyTeleportManagerState {
        unlockReceivedKey(receiver, response, "send $round")
        val received = awaitOnMain("send $round key") { receiver.manager.secrets.revealXprv() }
        assertEquals("send $round delivered a different key", FIXTURE_XPRV, received)

        receiver.manager.dispatch(KeyTeleportManagerAction.ImportReceivedWallet)
        return awaitOnMain("send $round import to finish") {
            receiver.manager.state.takeIf {
                it is KeyTeleportManagerState.ReceiveImportedWallet ||
                    it is KeyTeleportManagerState.ReceiveAlreadyImportedWallet
            }
        }
    }

    /**
     * Wave away the restored cloud backup's verification prompt with Not Now, as a person would
     *
     * Each wallet change asks to verify the updated backup, in a dialog over whatever screen is
     * showing. It has nothing to do with the screen under test but blocks it, so this waits until
     * no prompt has shown for [PROMPT_QUIET_MS], dismissing each one that does
     */
    private fun dismissCloudBackupVerificationPrompts() {
        val deadline = SystemClock.uptimeMillis() + PROMPT_SETTLE_TIMEOUT_MS
        var quietSince = SystemClock.uptimeMillis()

        while (SystemClock.uptimeMillis() - quietSince < PROMPT_QUIET_MS) {
            check(SystemClock.uptimeMillis() < deadline) { "the cloud backup verification prompt kept coming back" }

            if (onMainValue { CloudBackupManager.getInstance().rootPrompt } is CloudBackupRootPrompt.Verification) {
                device.clickText("Not Now")
                waitUntil("Not Now should dismiss the verification prompt") {
                    CloudBackupManager.getInstance().rootPrompt !is CloudBackupRootPrompt.Verification
                }
                quietSince = SystemClock.uptimeMillis()
            }

            Thread.sleep(SETTLE_POLL_MS)
        }
    }

    // the manager the KeyTeleport screen shows now, which a route change or relaunch replaces
    private fun receiveState(): KeyTeleportManagerState = AppManager.getInstance().getKeyTeleportManager().state

    private fun readPrivateKey(id: WalletId): String =
        runBlocking { AppManager.getInstance().getWalletManagerLoaded(id) }.exposeXprv()

    private fun walletIds(): List<WalletId> = Database().wallets().all().map { it.id }

    private fun walletsHoldingTheFixtureKey(): Int =
        Database().wallets().all().count { wallet ->
            runCatching { readPrivateKey(wallet.id) }.getOrNull() == FIXTURE_XPRV
        }

    private fun clipboard(): ClipboardManager =
        InstrumentationRegistry.getInstrumentation().targetContext.getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager

    private fun putOnClipboard(text: String) {
        onMain { clipboard().setPrimaryClip(ClipData.newPlainText("test", text)) }
    }

    private fun clearClipboard() {
        onMain { clipboard().clearPrimaryClip() }
    }

    private fun clipboardText(): String? =
        clipboard()
            .primaryClip
            ?.takeIf { it.itemCount > 0 }
            ?.getItemAt(0)
            ?.text
            ?.toString()

    private fun walletIdOrNull() =
        AppManager
            .getInstance()
            .wallets
            .firstOrNull { it.name == ScriptedCloudBackupFixture.XPRV_WALLET_NAME }
            ?.id

    private fun onMain(block: () -> Unit) {
        InstrumentationRegistry.getInstrumentation().runOnMainSync(block)
    }

    private fun <T : Any> onMainValue(read: () -> T): T = awaitOnMain("a value from the main thread", read = read)

    private fun waitUntil(
        message: String,
        timeoutMs: Long = 20_000L,
        condition: () -> Boolean,
    ) {
        val deadline = System.currentTimeMillis() + timeoutMs
        while (System.currentTimeMillis() < deadline) {
            var satisfied = false
            onMain { satisfied = condition() }
            if (satisfied) return

            Thread.sleep(100)
        }

        error(message)
    }

    private fun UiDevice.waitForLockScreen() {
        waitForText(LOCK_TITLE)
        waitForSettledNumberPad()
    }

    // the lock cover slides up from the bottom, so a key found mid-slide has moved by the time the
    // tap lands and the tap enters its neighbour; the bottom row stops moving once the cover is in place
    private fun UiDevice.waitForSettledNumberPad() {
        waitForSettled(By.res("numberPadPin.digit.0"), "the number pad")
    }

    /**
     * Waits until [selector] matches a node whose bounds held still between two looks, so a tap on
     * it lands where it was found rather than where a sliding screen or a scroll fling has moved it
     */
    private fun UiDevice.waitForSettled(
        selector: BySelector,
        description: String,
        timeoutMs: Long = 20_000L,
    ): UiObject2 {
        val deadline = System.currentTimeMillis() + timeoutMs
        var previous: Rect? = null

        while (System.currentTimeMillis() < deadline) {
            val node = findObject(selector)
            val bounds =
                try {
                    node?.visibleBounds
                } catch (_: StaleObjectException) {
                    null
                }

            if (node != null && bounds != null && bounds == previous) return node

            previous = bounds
            Thread.sleep(SETTLE_POLL_MS)
        }

        error("$description never held still, last bounds $previous\n${dumpWindowHierarchy()}")
    }

    private fun UiDevice.enterPin(pin: String) {
        pin.forEach { digit ->
            val key =
                wait(Until.findObject(By.res("numberPadPin.digit.$digit")), 5_000L)
                    ?: error("number pad key $digit is missing\n${dumpWindowHierarchy()}")
            key.click()
        }
        waitForIdle()
    }

    private fun UiDevice.assertNoKeyTeleportSendFor(durationMs: Long) {
        assertFalse("a KeyTeleport send started without the main PIN", wait(Until.hasObject(By.text(SEND_TITLE)), durationMs))
    }

    private fun UiDevice.assertPrivateKeyNotShown() {
        assertFalse("the private key was on screen", hasObject(By.textContains(FIXTURE_XPRV.take(16))))
    }

    private fun UiDevice.typeIntoField(value: String) {
        val field =
            wait(Until.findObject(By.clazz("android.widget.EditText")), 10_000L)
                ?: error("no text field on screen: ${visibleTexts()}")
        field.text = value
        waitForIdle()
    }

    // gives a reveal that would follow the PIN time to appear before asserting it never did
    private fun UiDevice.assertPrivateKeyHiddenFor(durationMs: Long) {
        assertFalse(
            "the private key was revealed",
            wait(Until.hasObject(By.text(REVEAL_TITLE)), durationMs) ||
                hasObject(By.textContains(FIXTURE_XPRV.take(16))),
        )
    }

    private fun UiDevice.waitForText(
        value: String,
        timeoutMs: Long = 20_000L,
    ): UiObject2 =
        wait(Until.findObject(By.text(value)), timeoutMs)
            ?: error("Timed out waiting for text \"$value\", on screen: ${visibleTexts()}")

    private fun UiDevice.visibleTexts(): List<String> = findObjects(By.textStartsWith("")).mapNotNull { it.text }

    private fun UiDevice.clickText(value: String) {
        waitForText(value)
        clickClickableAncestor(waitForSettled(By.text(value), "text \"$value\""))
    }

    private fun UiDevice.scrollUntilText(
        value: String,
        timeoutMs: Long = 20_000L,
    ): UiObject2 {
        val deadline = System.currentTimeMillis() + timeoutMs

        while (System.currentTimeMillis() < deadline) {
            if (hasObject(By.text(value))) return waitForSettled(By.text(value), "text \"$value\"")

            swipe(displayWidth / 2, displayHeight * 3 / 4, displayWidth / 2, displayHeight / 4, 20)
            Thread.sleep(250)
        }

        error("Timed out scrolling to text \"$value\"")
    }

    private fun UiDevice.clickClickableAncestor(node: UiObject2) {
        var clickable: UiObject2? = node

        while (clickable != null && !clickable.isClickable) {
            clickable = clickable.parent
        }

        requireNotNull(clickable) { "expected a clickable ancestor for ${node.text}" }.click()
    }

    private companion object {
        // BIP32 test vector 1 master key, the private key inside the fixture wallet
        const val FIXTURE_XPRV =
            "xprv9s21ZrQH143K3QTDL4LXw2F7HEK3wJUD2nW2nRk4stbPy6cq3jPPqjiChkVvvNKmPGJxWUtg6LnF5kejMRNNU3TGtRBeJgk33yuGBxrMPHi"
        const val MAIN_PIN = "135791"
        const val DECOY_PIN = "246802"
        const val WRONG_PIN = "999999"
        const val EXPORT_ITEM = "Export private key"
        const val LOCK_TITLE = "Enter Cove PIN"
        const val REVEAL_TITLE = "Extended Private Key"
        const val REVEAL_ACTION = "Reveal"
        const val KEY_TELEPORT_ACTION = "KeyTeleport"
        const val SEND_TITLE = "Send by KeyTeleport"
        const val XPRV_REVIEW_TITLE = "Extended private key received"
        const val GRACE_PERIOD_MS = 2_500L
        const val READ_ROUNDS = 25
        const val WRITE_ROUNDS = 3
        const val SEND_ROUNDS = 4
        const val REVEAL_SETTLE_MS = 3_000L
        const val SETTLE_POLL_MS = 200L
        const val PROMPT_QUIET_MS = 3_000L
        const val PROMPT_SETTLE_TIMEOUT_MS = 30_000L
        const val WALLET_SETTINGS_SECTION = "Wallet information"

        // published BIP39 test vector, never used for funds; the same bystander RepeatedSecretAccessTest uses
        val BYSTANDER_WORDS =
            listOf(
                "scheme", "spot", "photo", "card", "baby", "mountain",
                "device", "kick", "cradle", "pact", "join", "borrow",
            )
        val BYSTANDER_WALLET = TestWalletWords(BYSTANDER_WORDS, masterFingerprint = "b760aecd")
    }
}
