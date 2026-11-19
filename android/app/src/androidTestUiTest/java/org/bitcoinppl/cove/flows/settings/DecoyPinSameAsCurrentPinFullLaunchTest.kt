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
import org.bitcoinppl.cove.AuthManager
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
import org.bitcoinppl.cove_core.AuthManagerAction
import org.bitcoinppl.cove_core.AuthType
import org.bitcoinppl.cove_core.BootstrapStep
import org.bitcoinppl.cove_core.RustAuthManager
import org.bitcoinppl.cove_core.bootstrapProgress
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assume.assumeTrue
import org.junit.Before
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

/**
 * Choosing the app PIN as the decoy PIN in Settings shows the reason in English
 *
 * The alert used to show the Kotlin message of the structured UniFFI error, a field dump such as
 * `v1=org.bitcoinppl.cove_core.TrickPinException$SameAsCurrentPin:`. Runs in the uiTest flavour
 * against the real app; needs the fake node up and forwarded, which `just android-instrumented-ci` does
 */
@LargeTest
@RunWith(AndroidJUnit4::class)
class DecoyPinSameAsCurrentPinFullLaunchTest {
    @get:Rule
    val stayAwake = AndroidDeviceStayAwakeRule()

    private lateinit var device: UiDevice
    private lateinit var wallet: FullLaunchWalletRobot
    private val node = FakeNodeControl()
    private var previousShowImeWithHardKeyboard: String? = null

    // set once this test dispatches its PIN, so a skipped or early-failing test never removes a lock it found
    private var pinSet = false

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
            if (pinSet) restoreNoAppLock()
        } finally {
            when (val previous = previousShowImeWithHardKeyboard) {
                null, "null" -> device.executeShellCommand("settings delete secure $SHOW_IME_SETTING")
                else -> device.executeShellCommand("settings put secure $SHOW_IME_SETTING $previous")
            }
        }
    }

    @Test
    fun decoyPinSameAsTheAppPinShowsTheReasonInEnglish() {
        launchPastOnboarding()
        setAppPin()
        openSettings()

        device.clickSwitchInRowOf(DECOY_TOGGLE_TITLE)
        device.clickText("Yes, Enable Decoy PIN")

        device.waitFor(By.text("Enter Decoy PIN"))
        device.enterPin(PIN)
        device.waitFor(By.text("Confirm Decoy PIN"))
        device.enterPin(PIN)

        val shownMessage = device.alertMessage(ALERT_TITLE)
        println("decoy PIN alert shows: \"$shownMessage\"")

        assertFalse("the alert shows a UniFFI field dump: \"$shownMessage\"", shownMessage.contains("v1="))
        assertEquals(EXPECTED_MESSAGE, shownMessage)
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

    // the launch resets the app's data, but that reset only logs a failure, so a lock someone relies
    // on can survive it; the test sets and removes its own PIN, so it never runs over one
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

    // the bug is in the decoy PIN alert, so the app PIN is set in process, as Settings → Enable PIN would
    private fun setAppPin() {
        val instrumentation = InstrumentationRegistry.getInstrumentation()
        instrumentation.runOnMainSync { AuthManager.getInstance().dispatch(AuthManagerAction.SetPin(PIN)) }
        pinSet = true

        wallet.waitUntil("the app PIN never enabled the app lock") {
            var enabled = false
            instrumentation.runOnMainSync { enabled = AuthManager.getInstance().isAuthEnabled }
            enabled
        }
    }

    private fun openSettings() {
        device.clickSettled(By.desc("Menu"), "the wallet's menu button")

        // the sidebar slides in, so the row is tapped only once it has stopped moving
        device.clickText("Settings")
        device.scrollTo(By.text(DECOY_TOGGLE_TITLE))
    }

    private companion object {
        const val PIN = "135790"
        const val DECOY_TOGGLE_TITLE = "Enable Decoy PIN"
        const val ALERT_TITLE = "Something went wrong!"
        const val EXPECTED_MESSAGE = "Unable to set the decoy PIN, because it is the same as your current PIN"
        const val SHOW_IME_SETTING = "show_ime_with_hard_keyboard"
    }
}

// the switch is the row's trailing content, not a child of its title, so it is the checkable node
// laid out level with the title
private fun UiDevice.clickSwitchInRowOf(title: String) {
    val titleBounds = waitForSettled(By.text(title), "the \"$title\" row").visibleBounds
    val switch =
        findObjects(By.checkable(true)).singleOrNull { it.visibleCenter.y in titleBounds.top..titleBounds.bottom }
            ?: error("no single switch level with \"$title\"\n${hierarchy()}")

    switch.click()
}

// the decoy PIN pad slides up as a sheet, and a key found mid-slide sends the tap to its neighbour,
// so the bottom row must hold still first. The pad checks a full PIN after a short delay, which the
// callers wait out by waiting for the next screen
private fun UiDevice.enterPin(pin: String) {
    waitForSettled(pinKey('0'), "the number pad")

    pin.forEach { digit -> waitFor(pinKey(digit), timeoutMs = 5_000L).click() }
}

// the sheet is a dialog window of its own, which doesn't expose the pad's test tags as resource
// ids, so a key is the clickable node holding its digit
private fun pinKey(digit: Char): BySelector = By.clickable(true).hasChild(By.text(digit.toString()))

// everything the alert shows besides its title and button, so a failure prints what the user saw
private fun UiDevice.alertMessage(alertTitle: String): String {
    val title = waitFor(By.text(alertTitle))
    var dialog: UiObject2 = title
    while (!dialog.hasObject(By.text("OK"))) {
        dialog = dialog.parent ?: error("the alert has no OK button\n${hierarchy()}")
    }

    return dialog
        .findObjects(By.textStartsWith(""))
        .mapNotNull { it.text }
        .filterNot { it == alertTitle || it == "OK" }
        .joinToString("\n")
}

private fun UiDevice.clickText(value: String) {
    clickSettled(By.text(value), "text \"$value\"")
}

private fun UiDevice.clickSettled(
    selector: BySelector,
    description: String,
) {
    waitFor(selector)
    var clickable: UiObject2? = waitForSettled(selector, description)
    while (clickable != null && !clickable.isClickable) {
        clickable = clickable.parent
    }

    requireNotNull(clickable) { "expected a clickable ancestor for $description" }.click()
}

/**
 * Waits until [selector] matches a node whose bounds held still between two looks, so a tap on
 * it lands where it was found rather than where a sliding screen has moved it
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

    error("$description never held still, last bounds $previous\n${hierarchy()}")
}

private const val SETTLE_POLL_MS = 200L
