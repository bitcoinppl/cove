package org.bitcoinppl.cove.flows.settings

import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.filters.LargeTest
import androidx.test.platform.app.InstrumentationRegistry
import androidx.test.uiautomator.By
import androidx.test.uiautomator.UiDevice
import androidx.test.uiautomator.Until
import kotlinx.coroutines.runBlocking
import org.bitcoinppl.cove.AppManager
import org.bitcoinppl.cove.AuthManager
import org.bitcoinppl.cove.UnlockMode
import org.bitcoinppl.cove.test.AndroidDeviceStayAwakeRule
import org.bitcoinppl.cove.test.FakeNodeControl
import org.bitcoinppl.cove.test.FullLaunchOnboardingRobot
import org.bitcoinppl.cove.test.FullLaunchStartupRobot
import org.bitcoinppl.cove.test.FullLaunchWalletRobot
import org.bitcoinppl.cove.test.fullLaunchDevice
import org.bitcoinppl.cove.test.hierarchy
import org.bitcoinppl.cove.test.launchFullApp
import org.bitcoinppl.cove.test.restoreNoAppLock
import org.bitcoinppl.cove.test.scrollTo
import org.bitcoinppl.cove.test.waitFor
import org.bitcoinppl.cove_core.AuthType
import org.bitcoinppl.cove_core.BootstrapStep
import org.bitcoinppl.cove_core.Route
import org.bitcoinppl.cove_core.RustAuthManager
import org.bitcoinppl.cove_core.SettingsRoute
import org.bitcoinppl.cove_core.bootstrapProgress
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Assume.assumeTrue
import org.junit.Before
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

/**
 * Decoy mode's Settings must look like main mode's, so turning off a setting main mode guards
 * with its PIN asks for the decoy PIN, and only decoy mode's own toggles change
 *
 * Runs in the uiTest flavour against the real app; needs the fake node up and forwarded, which
 * `just android-instrumented-ci` does
 */
@LargeTest
@RunWith(AndroidJUnit4::class)
class DecoySecuritySettingsFullLaunchTest {
    @get:Rule
    val stayAwake = AndroidDeviceStayAwakeRule()

    private lateinit var device: UiDevice
    private lateinit var wallet: FullLaunchWalletRobot
    private val node = FakeNodeControl()
    private var previousShowImeWithHardKeyboard: String? = null

    // set once this test sets its PINs, so a skipped or early-failing test never removes a lock it found
    private var pinsSet = false

    @Before
    fun prepareDevice() {
        device = fullLaunchDevice()
        wallet = FullLaunchWalletRobot(device)

        // see UtxoLockFullLaunchTest: the onboarding robot must never see the soft keyboard
        previousShowImeWithHardKeyboard = device.executeShellCommand("settings get secure $SHOW_IME_SETTING").trim()
        device.executeShellCommand("settings put secure $SHOW_IME_SETTING 0")
    }

    @After
    fun cleanUp() {
        try {
            if (pinsSet) restoreNoAppLock()
        } finally {
            when (val previous = previousShowImeWithHardKeyboard) {
                null, "null" -> device.executeShellCommand("settings delete secure $SHOW_IME_SETTING")
                else -> device.executeShellCommand("settings put secure $SHOW_IME_SETTING $previous")
            }
        }
    }

    @Test
    fun decoyModeAsksForTheDecoyPinToTurnSettingsOff() {
        launchPastOnboarding()
        setMainAndDecoyPins()
        enterDecoyMode()
        openSettings()

        assertFalse("decoy mode starts with the wipe data PIN on", device.switchInRowOf(WIPE_TOGGLE_TITLE).isChecked)
        device.clickSwitchInRowOf(WIPE_TOGGLE_TITLE)
        device.clickText("Yes, Enable Wipe Data PIN")
        device.waitFor(By.text("Enter Wipe Data PIN"))
        device.enterPin(DECOY_WIPE_DATA_PIN)
        device.waitFor(By.text("Confirm Wipe Data PIN"))
        device.enterPin(DECOY_WIPE_DATA_PIN)
        awaitSwitch(WIPE_TOGGLE_TITLE, checked = true, "turning the wipe data PIN on in decoy mode didn't show it on")
        turnOffWithDecoyPin(WIPE_TOGGLE_TITLE)

        device.clickSwitchInRowOf(DECOY_TOGGLE_TITLE)
        device.clickText("Yes, Enable Decoy PIN")
        device.waitFor(By.text("Enter Decoy PIN"))
        device.enterPin(DECOY_WIPE_DATA_PIN)
        device.waitFor(By.text("Confirm Decoy PIN"))
        device.enterPin(DECOY_WIPE_DATA_PIN)
        awaitSwitch(DECOY_TOGGLE_TITLE, checked = true, "turning the decoy PIN on in decoy mode didn't show it on")
        turnOffWithDecoyPin(DECOY_TOGGLE_TITLE)

        turnOffWithDecoyPin(PIN_TOGGLE_TITLE)

        // only decoy mode's toggles changed, so the saved lock is the one the test set up
        val (authType, hasDecoyPin, hasWipeDataPin) =
            RustAuthManager().use { rust ->
                Triple(rust.authType(), rust.isDecoyPinEnabled(), rust.isWipeDataPinEnabled())
            }

        assertEquals("decoy mode changed the saved app lock", AuthType.PIN, authType)
        assertTrue("decoy mode turned off the saved decoy PIN", hasDecoyPin)
        assertFalse("decoy mode saved a wipe data PIN", hasWipeDataPin)
    }

    private fun turnOffWithDecoyPin(title: String) {
        device.clickSwitchInRowOf(title)

        val prompt = By.text(CURRENT_PIN_TITLE)
        if (!device.wait(Until.hasObject(prompt), PROMPT_TIMEOUT_MS)) {
            error("turning off \"$title\" in decoy mode asked for no PIN\n${device.hierarchy()}")
        }

        device.enterPin(DECOY_PIN)
        if (!device.wait(Until.gone(prompt), PROMPT_TIMEOUT_MS)) {
            error("the decoy PIN didn't turn off \"$title\" in decoy mode\n${device.hierarchy()}")
        }

        awaitSwitch(title, checked = false, "turning off \"$title\" in decoy mode left it on")
    }

    private fun awaitSwitch(
        title: String,
        checked: Boolean,
        message: String,
    ) {
        device.scrollTo(By.text(title))
        wallet.waitUntil(message) { device.switchInRowOf(title).isChecked == checked }
    }

    private fun launchPastOnboarding() {
        node.reset()

        launchFullApp()
        awaitBootstrap()
        assumeNoAppLock()

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

        device.waitFor(By.res("selectedWallet.balance"), timeoutMs = 30_000L)
    }

    private fun awaitBootstrap() {
        wallet.waitUntil("the app never finished bootstrap", timeoutMs = 60_000L) {
            bootstrapProgress() == BootstrapStep.COMPLETE
        }
    }

    // see DecoyPinSameAsCurrentPinFullLaunchTest: the test sets and removes its own PINs, so it
    // never runs over a lock it found
    private fun assumeNoAppLock() {
        val (authType, hasDecoyPin, hasWipeDataPin) =
            RustAuthManager().use { rust ->
                Triple(rust.authType(), rust.isDecoyPinEnabled(), rust.isWipeDataPinEnabled())
            }

        assumeTrue(
            "an app lock is already set up on this device",
            authType == AuthType.NONE && !hasDecoyPin && !hasWipeDataPin,
        )
    }

    // the rules under test are decoy mode's, so the PINs are set in process as Settings would
    private fun setMainAndDecoyPins() {
        val auth = AuthManager.getInstance()
        onMain {
            auth.setPin(MAIN_PIN)
            auth.setDecoyPin(DECOY_PIN)
        }
        pinsSet = true

        wallet.waitUntil("the PINs never enabled the app lock") {
            var ready = false
            onMain { ready = auth.isAuthEnabled && auth.isDecoyPinEnabled }
            ready
        }
    }

    // the decoy PIN at the lock screen takes this path, which leaves the main wallets behind
    private fun enterDecoyMode() {
        var mode: UnlockMode? = null
        onMain { mode = runBlocking { AuthManager.getInstance().handleAndReturnUnlockMode(DECOY_PIN) } }
        assertEquals("the decoy PIN didn't unlock decoy mode", UnlockMode.DECOY, mode)

        // decoy mode has no wallets yet, so the app leaves the main wallet's screen
        wallet.waitUntil("the app never left the main wallet") {
            var onWallet = true
            onMain { onWallet = AppManager.getInstance().router.default is Route.SelectedWallet }
            !onWallet
        }
    }

    private fun openSettings() {
        onMain { AppManager.getInstance().resetRoute(Route.Settings(SettingsRoute.Main)) }
        device.scrollTo(By.text(WIPE_TOGGLE_TITLE))
    }

    private fun onMain(block: () -> Unit) {
        InstrumentationRegistry.getInstrumentation().runOnMainSync(block)
    }

    private companion object {
        const val MAIN_PIN = "135791"
        const val DECOY_PIN = "246802"
        const val DECOY_WIPE_DATA_PIN = "112233"
        const val PIN_TOGGLE_TITLE = "Enable PIN"
        const val WIPE_TOGGLE_TITLE = "Enable Wipe Data PIN"
        const val DECOY_TOGGLE_TITLE = "Enable Decoy PIN"
        const val CURRENT_PIN_TITLE = "Enter Current PIN"
        const val PROMPT_TIMEOUT_MS = 5_000L
        const val SHOW_IME_SETTING = "show_ime_with_hard_keyboard"
    }
}
