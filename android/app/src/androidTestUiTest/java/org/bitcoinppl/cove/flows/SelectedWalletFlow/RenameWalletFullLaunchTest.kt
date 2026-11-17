package org.bitcoinppl.cove.flows.SelectedWalletFlow

import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.filters.LargeTest
import androidx.test.uiautomator.By
import androidx.test.uiautomator.StaleObjectException
import androidx.test.uiautomator.UiDevice
import org.bitcoinppl.cove.test.AndroidDeviceStayAwakeRule
import org.bitcoinppl.cove.test.FakeNodeControl
import org.bitcoinppl.cove.test.FullLaunchOnboardingRobot
import org.bitcoinppl.cove.test.FullLaunchStartupRobot
import org.bitcoinppl.cove.test.FullLaunchWalletRobot
import org.bitcoinppl.cove.test.findText
import org.bitcoinppl.cove.test.fullLaunchDevice
import org.bitcoinppl.cove.test.launchFullApp
import org.bitcoinppl.cove.test.waitFor
import org.junit.After
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

/**
 * Opens the rename screen from the wallet title's long-press menu, checks its name field keeps
 * focus, and types into it at key-event speed
 *
 * The rename screen focuses its name field as it appears with the cursor after the name, where the
 * field used to open with it before the name, and fast typing must land whole while each edit is
 * saved and reconciled back. Runs in the uiTest flavour; needs the fake node up and forwarded, which
 * `just android-instrumented-ci` does
 */
@LargeTest
@RunWith(AndroidJUnit4::class)
class RenameWalletFullLaunchTest {
    private companion object {
        const val TITLE = "selectedWallet.title"
        const val NAME_FIELD = "walletSettings.changeName.field"
        const val SUFFIX = " Renamed"

        const val FOCUS_TIMEOUT_MS = 5_000L

        // the iOS rename screen used to lose its focus within a second of opening, so focus must
        // outlast that here too
        const val FOCUS_HOLD_MS = 3_000L
        const val TYPED_TIMEOUT_MS = 5_000L
    }

    @get:Rule
    val stayAwake = AndroidDeviceStayAwakeRule()

    private lateinit var device: UiDevice
    private lateinit var wallet: FullLaunchWalletRobot
    private val node = FakeNodeControl()
    private var previousShowImeWithHardKeyboard: String? = null

    @Before
    fun prepareDevice() {
        device = fullLaunchDevice()
        wallet = FullLaunchWalletRobot(device)

        // see UtxoLockFullLaunchTest: the onboarding robot must never see the soft keyboard. The
        // field's focus, not the keyboard, is what this test reads
        previousShowImeWithHardKeyboard = device.executeShellCommand("settings get secure show_ime_with_hard_keyboard").trim()
        device.executeShellCommand("settings put secure show_ime_with_hard_keyboard 0")
    }

    @After
    fun restoreDevice() {
        when (val previous = previousShowImeWithHardKeyboard) {
            null, "null" -> device.executeShellCommand("settings delete secure show_ime_with_hard_keyboard")
            else -> device.executeShellCommand("settings put secure show_ime_with_hard_keyboard $previous")
        }
    }

    @Test
    fun renameFromTitleMenuKeepsFocusAndEveryTypedKey() {
        launchPastOnboarding()

        device.waitFor(By.res(TITLE)).longClick()
        device.waitFor(By.text("Change Name")).click()

        val originalName = device.waitFor(By.res(NAME_FIELD)).text.orEmpty()
        wallet.waitUntil("rename screen never focused the name field", timeoutMs = FOCUS_TIMEOUT_MS) {
            nameFieldIsFocused()
        }

        val focusHoldEnds = System.currentTimeMillis() + FOCUS_HOLD_MS
        while (System.currentTimeMillis() < focusHoldEnds) {
            assertTrue("the name field lost focus on its own after the rename screen opened", nameFieldIsFocused())
            Thread.sleep(100)
        }

        // `input text` sends one key event per character with no pause between them, faster than
        // each edit's save round-trips, and needs %s for a space
        device.executeShellCommand("input text ${SUFFIX.replace(" ", "%s")}")

        val expected = originalName + SUFFIX
        wallet.waitUntil("expected the name field to hold '$expected' after typing", timeoutMs = TYPED_TIMEOUT_MS) {
            device.findText(By.res(NAME_FIELD)) == expected
        }
    }

    private fun nameFieldIsFocused(): Boolean =
        try {
            device.findObject(By.res(NAME_FIELD))?.isFocused == true
        } catch (_: StaleObjectException) {
            false
        }

    private fun launchPastOnboarding() {
        node.reset()

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

        device.waitFor(By.res("selectedWallet.balance"), timeoutMs = 30_000L)
    }
}
