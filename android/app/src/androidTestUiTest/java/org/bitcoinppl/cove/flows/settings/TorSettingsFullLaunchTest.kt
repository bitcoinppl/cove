package org.bitcoinppl.cove.flows.settings

import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.filters.LargeTest
import androidx.test.uiautomator.By
import androidx.test.uiautomator.StaleObjectException
import androidx.test.uiautomator.UiDevice
import androidx.test.uiautomator.Until
import org.bitcoinppl.cove.test.AndroidDeviceStayAwakeRule
import org.bitcoinppl.cove.test.FakeNodeControl
import org.bitcoinppl.cove.test.FullLaunchOnboardingRobot
import org.bitcoinppl.cove.test.FullLaunchStartupRobot
import org.bitcoinppl.cove.test.FullLaunchWalletRobot
import org.bitcoinppl.cove.test.findText
import org.bitcoinppl.cove.test.fullLaunchDevice
import org.bitcoinppl.cove.test.hierarchy
import org.bitcoinppl.cove.test.launchFullApp
import org.bitcoinppl.cove.test.scrollTo
import org.bitcoinppl.cove.test.waitFor
import org.junit.After
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

/**
 * Settings → Tor: a bad proxy address shows its error under the field, a good one saves and Tor
 * reports it is using the proxy, and Off turns Tor off again
 *
 * Nothing listens on 127.0.0.1:9, which doesn't matter here: saving a proxy publishes its status
 * without connecting. Runs in the uiTest flavour against the real app; needs the fake node up and
 * forwarded, which `just android-instrumented-ci` does
 */
@LargeTest
@RunWith(AndroidJUnit4::class)
class TorSettingsFullLaunchTest {
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

        // see UtxoLockFullLaunchTest: the onboarding robot must never see the soft keyboard, and
        // here it would also cover Use Proxy
        previousShowImeWithHardKeyboard = device.executeShellCommand("settings get secure $SHOW_IME_SETTING").trim()
        device.executeShellCommand("settings put secure $SHOW_IME_SETTING 0")
    }

    @After
    fun restoreDevice() {
        when (val previous = previousShowImeWithHardKeyboard) {
            null, "null" -> device.executeShellCommand("settings delete secure $SHOW_IME_SETTING")
            else -> device.executeShellCommand("settings put secure $SHOW_IME_SETTING $previous")
        }
    }

    @Test
    fun proxyAddressIsValidatedSavedAndTurnedOff() {
        launchPastOnboarding()
        openSettings()

        val torRow = device.scrollTo(By.res(TOR_ROW))
        assertTrue(
            "expected the Tor row to show Off\n${device.hierarchy()}",
            torRow.hasObject(By.text("Off")),
        )
        device.clickSettled(By.res(TOR_ROW), "the Tor row")
        waitForStatus("Off")

        device.clickSettled(By.res(MODE_PROXY), "the Tor proxy option")
        enterProxyAddress("not a proxy")
        device.clickSettled(By.res(USE_PROXY), "Use Proxy")
        device.waitFor(By.res(PROXY_ERROR))

        enterProxyAddress("127.0.0.1:9")
        device.clickSettled(By.res(USE_PROXY), "Use Proxy")
        waitForStatus("Using proxy")
        assertTrue(
            "the proxy error stayed after a valid address was saved",
            device.wait(Until.gone(By.res(PROXY_ERROR)), UI_TIMEOUT_MS),
        )

        device.clickSettled(By.res(MODE_OFF), "the Off option")
        waitForStatus("Off")
    }

    private fun waitForStatus(expected: String) {
        device.scrollTo(By.res(STATUS))
        wallet.waitUntil("expected the Tor status to read \"$expected\"", timeoutMs = UI_TIMEOUT_MS) {
            device.findText(By.res(STATUS)) == expected
        }
    }

    // a recomposition can replace the field between finding and typing into it, so this keeps
    // setting the text on whichever node is current until the field holds it
    private fun enterProxyAddress(text: String) {
        device.waitFor(By.res(PROXY_FIELD))
        wallet.waitUntil("expected the proxy address field to hold \"$text\"", timeoutMs = UI_TIMEOUT_MS) {
            try {
                val field = device.findObject(By.res(PROXY_FIELD)) ?: return@waitUntil false
                if (field.text != text) field.text = text
                device.findText(By.res(PROXY_FIELD)) == text
            } catch (_: StaleObjectException) {
                false
            }
        }
    }

    private fun openSettings() {
        device.clickSettled(By.desc("Menu"), "the wallet's menu button")

        // the sidebar slides in, so the row is tapped only once it has stopped moving
        device.clickText("Settings")
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

    private companion object {
        const val TOR_ROW = "settings.tor"
        const val MODE_OFF = "tor.mode.off"
        const val MODE_PROXY = "tor.mode.proxy"
        const val PROXY_FIELD = "tor.proxy.address"
        const val PROXY_ERROR = "tor.proxy.error"
        const val USE_PROXY = "tor.proxy.use"
        const val STATUS = "tor.status"
        const val UI_TIMEOUT_MS = 10_000L
        const val SHOW_IME_SETTING = "show_ime_with_hard_keyboard"
    }
}
