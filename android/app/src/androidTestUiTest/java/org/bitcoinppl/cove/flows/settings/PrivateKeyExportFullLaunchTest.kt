package org.bitcoinppl.cove.flows.settings

import android.graphics.Rect
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.filters.LargeTest
import androidx.test.platform.app.InstrumentationRegistry
import androidx.test.uiautomator.By
import androidx.test.uiautomator.BySelector
import androidx.test.uiautomator.StaleObjectException
import androidx.test.uiautomator.UiDevice
import androidx.test.uiautomator.UiObject2
import androidx.test.uiautomator.Until
import org.bitcoinppl.cove.AppManager
import org.bitcoinppl.cove.AuthManager
import org.bitcoinppl.cove.test.FullLaunchOnboardingRobot
import org.bitcoinppl.cove.test.FullLaunchTestRule
import org.bitcoinppl.cove.test.ManualFullLaunchTest
import org.bitcoinppl.cove.test.dumpWindowHierarchy
import org.bitcoinppl.cove.test.fullLaunchDevice
import org.bitcoinppl.cove.test.launchFullApp
import org.bitcoinppl.cove.test.restoreNoAppLock
import org.bitcoinppl.cove.testconfig.ScriptedCloudBackupFixture
import org.bitcoinppl.cove.testconfig.ScriptedCloudStorageAccess
import org.bitcoinppl.cove.testconfig.ScriptedPasskeyProvider
import org.bitcoinppl.cove_core.AppLockReason
import org.bitcoinppl.cove_core.AuthManagerAction
import org.bitcoinppl.cove_core.Route
import org.bitcoinppl.cove_core.SettingsRoute
import org.bitcoinppl.cove_core.WalletSettingsRoute
import org.bitcoinppl.cove_core.types.WalletId
import org.junit.After
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

/**
 * Export Private Key on a wallet that holds only an extended private key
 *
 * The wallet comes from restoring the scripted cloud backup, the same restore a person runs after
 * reinstalling, so no test-only path creates it. The export waits for the real lock screen
 */
@ManualFullLaunchTest
@LargeTest
@RunWith(AndroidJUnit4::class)
class PrivateKeyExportFullLaunchTest {
    private lateinit var device: UiDevice

    // leaving decoy mode reloads the wallet list, so the id is kept from the restore
    private lateinit var walletId: WalletId

    @get:Rule
    val fullLaunch = FullLaunchTestRule()

    @Before
    fun prepareDevice() {
        ScriptedPasskeyProvider.reset()
        device = fullLaunchDevice()
    }

    // the next test may host LockView without relaunching, and refuses to run over a leftover PIN
    @After
    fun removeAppLock() {
        restoreNoAppLock()
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
        val auth = AuthManager.getInstance()
        onMain { auth.dispatch(AuthManagerAction.SetPin(MAIN_PIN)) }
        waitUntil("the main PIN should enable the app lock") { auth.isAuthEnabled }
        onMain { auth.setDecoyPin(DECOY_PIN) }
        waitUntil("the decoy PIN should be saved") { auth.isDecoyPinEnabled }
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
        device.clickClickableAncestor(device.scrollUntilText(EXPORT_ITEM))
        device.waitForText("Are you sure?")
        device.clickText("Continue")
        device.waitForText("Export Private Key")
        device.clickText("Reveal")
        device.waitForLockScreen()
    }

    private fun walletIdOrNull() =
        AppManager
            .getInstance()
            .wallets
            .firstOrNull { it.name == ScriptedCloudBackupFixture.XPRV_WALLET_NAME }
            ?.id

    private fun onMain(block: () -> Unit) {
        InstrumentationRegistry.getInstrumentation().runOnMainSync(block)
    }

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
        const val REVEAL_SETTLE_MS = 3_000L
        const val SETTLE_POLL_MS = 200L
        const val WALLET_SETTINGS_SECTION = "Wallet information"
    }
}
