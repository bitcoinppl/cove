package org.bitcoinppl.cove

import androidx.compose.material3.SnackbarHostState
import androidx.compose.runtime.remember
import androidx.compose.ui.test.junit4.StateRestorationTester
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithText
import androidx.test.platform.app.InstrumentationRegistry
import androidx.test.uiautomator.By
import androidx.test.uiautomator.UiDevice
import androidx.test.uiautomator.Until
import org.bitcoinppl.cove.test.bootstrapRustRuntimeForUiTest
import org.bitcoinppl.cove_core.AppAlertState
import org.bitcoinppl.cove_core.AppLockReason
import org.bitcoinppl.cove_core.AuthManagerAction
import org.junit.After
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Rule
import org.junit.Test

class MainActivityAppShellLockTest {
    @get:Rule
    val compose = createComposeRule()

    private val instrumentation = InstrumentationRegistry.getInstrumentation()

    @Before
    fun enablePin() {
        bootstrapRustRuntimeForUiTest()
        instrumentation.runOnMainSync { Auth.dispatch(AuthManagerAction.SetPin(PIN)) }
        compose.waitUntil(timeoutMillis = 5_000) { Auth.isAuthEnabled }
    }

    @After
    fun disablePin() {
        instrumentation.runOnMainSync {
            App.alertState = null
            Auth.dispatch(AuthManagerAction.DisablePin)
            Auth.unlockIfAuthDisabled()
        }
    }

    @Test
    fun appAlertStaysUnderTheLockWhenTheActivityIsRecreated() {
        val restoration = StateRestorationTester(compose)
        restoration.setContent {
            MainActivityAppShell(
                app = App,
                auth = Auth,
                snackbarHostState = remember { SnackbarHostState() },
                startupMode = StartupMode.ONBOARDING,
                onboardingManager = null,
                isPrivacyCoverVisible = false,
                onOnboardingComplete = {},
            )
        }

        // an app alert is up when the app locks
        compose.runOnIdle { App.alertState = TaggedItem(AppAlertState.InvalidWordGroup) }
        compose.waitForIdle()
        compose.runOnIdle { Auth.lock(AppLockReason.BACKGROUND) }
        compose.onNodeWithText("Enter Cove PIN").assertExists()

        // recreation opens every window again, in composition order
        restoration.emulateSavedInstanceStateRestore()

        // uiautomator only sees the active window, so this fails if the alert covers the PIN
        val device = UiDevice.getInstance(instrumentation)
        assertTrue(device.wait(Until.hasObject(By.res("numberPadPin.digit.1")), 5_000))
    }

    private companion object {
        const val PIN = "111111"
    }
}
