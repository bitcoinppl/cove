package org.bitcoinppl.cove.views

import android.net.Uri
import androidx.activity.compose.LocalActivityResultRegistryOwner
import androidx.activity.compose.ManagedActivityResultLauncher
import androidx.activity.result.ActivityResultRegistry
import androidx.activity.result.ActivityResultRegistryOwner
import androidx.activity.result.contract.ActivityResultContract
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.test.junit4.StateRestorationTester
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.core.app.ActivityOptionsCompat
import androidx.test.platform.app.InstrumentationRegistry
import androidx.test.uiautomator.By
import androidx.test.uiautomator.UiDevice
import androidx.test.uiautomator.Until
import org.bitcoinppl.cove.Auth
import org.bitcoinppl.cove.flows.SettingsFlow.FullScreenSettingsModal
import org.bitcoinppl.cove.test.bootstrapRustRuntimeForUiTest
import org.bitcoinppl.cove.test.restoreNoAppLock
import org.bitcoinppl.cove.ui.theme.CoveTheme
import org.bitcoinppl.cove_core.AppLockReason
import org.bitcoinppl.cove_core.AuthManagerAction
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Rule
import org.junit.Test

class LockAwareActivityResultTest {
    @get:Rule
    val compose = createComposeRule()

    private val instrumentation = InstrumentationRegistry.getInstrumentation()
    private val pickedFile = Uri.parse("content://org.bitcoinppl.cove.test/signed.psbt")
    private val picker = RegistryReturning(pickedFile)
    private val handled = mutableListOf<Uri?>()
    private lateinit var launcher: ManagedActivityResultLauncher<String, Uri?>

    @Before
    fun enablePin() {
        bootstrapRustRuntimeForUiTest()
        instrumentation.runOnMainSync { Auth.dispatch(AuthManagerAction.SetPin(PIN)) }
        compose.waitUntil(timeoutMillis = 5_000) { Auth.isAuthEnabled }
    }

    @After
    fun removeAppLock() {
        restoreNoAppLock()
    }

    @Test
    fun pickerResultFromAModalIsHandledAfterThePin() {
        compose.setContent {
            LockedApp {
                // backup export, import and verify launch their pickers from this modal window
                FullScreenSettingsModal(onDismiss = {}) { PickerHost() }
            }
        }

        returnFromPickerLocked()

        // uiautomator only sees the active window, so this fails if the modal still covers the PIN
        val device = UiDevice.getInstance(instrumentation)
        assertTrue(device.wait(Until.hasObject(By.res("numberPadPin.digit.1")), 5_000))

        enterPinAndExpectTheResult()
    }

    @Test
    fun heldPickerResultSurvivesActivityRecreationBeforeThePin() {
        val restoration = StateRestorationTester(compose)
        restoration.setContent { LockedApp { PickerHost() } }

        returnFromPickerLocked()

        // android won't deliver the result again if the activity is recreated before the PIN
        restoration.emulateSavedInstanceStateRestore()
        compose.runOnIdle { assertEquals(emptyList<Uri?>(), handled) }

        enterPinAndExpectTheResult()
    }

    @Test
    fun heldPickerResultIsDroppedWhenTheUnlockSwitchesMode() {
        instrumentation.runOnMainSync { Auth.setDecoyPin(DECOY_PIN) }
        val restoration = StateRestorationTester(compose)
        restoration.setContent { LockedApp { PickerHost() } }

        // picked for the main wallets, unlocked into decoy mode
        returnFromPickerLocked()
        restoration.emulateSavedInstanceStateRestore()
        unlockWith(DECOY_PIN)
        compose.runOnIdle {
            assertTrue(Auth.isInDecoyMode())
            assertEquals(emptyList<Uri?>(), handled)
        }

        // picked for the decoy wallets, unlocked back into main mode
        returnFromPickerLocked()
        restoration.emulateSavedInstanceStateRestore()
        unlockWith(PIN)
        compose.runOnIdle {
            assertFalse(Auth.isInDecoyMode())
            assertEquals(emptyList<Uri?>(), handled)
        }
    }

    @Composable
    private fun LockedApp(content: @Composable () -> Unit) {
        CompositionLocalProvider(LocalActivityResultRegistryOwner provides picker) {
            CoveTheme { LockView(content) }
        }
    }

    @Composable
    private fun PickerHost() {
        launcher =
            rememberLockAwareLauncherForActivityResult(ActivityResultContracts.GetContent()) {
                handled += it
            }

        Text("Picker host")
    }

    private fun returnFromPickerLocked() {
        // leaving for the system picker locks the app before the result comes back
        compose.runOnIdle {
            Auth.lock(AppLockReason.BACKGROUND)
            launcher.launch("*/*")
        }

        compose.onNodeWithText("Enter Cove PIN").assertExists()
        compose.runOnIdle { assertEquals(emptyList<Uri?>(), handled) }
    }

    private fun unlockWith(pin: String) {
        pin.forEach { digit -> compose.onNodeWithTag("numberPadPin.digit.$digit").performClick() }
        compose.waitUntil(timeoutMillis = 5_000) { !Auth.isLocked }
    }

    private fun enterPinAndExpectTheResult() {
        unlockWith(PIN)
        compose.waitUntil(timeoutMillis = 5_000) { handled.isNotEmpty() }
        compose.runOnIdle { assertEquals(listOf<Uri?>(pickedFile), handled) }
    }

    private companion object {
        const val PIN = "111111"
        const val DECOY_PIN = "222222"
    }
}

/** Stands in for the system picker by returning a fixed result as soon as it is launched */
private class RegistryReturning(private val result: Uri) : ActivityResultRegistryOwner {
    override val activityResultRegistry =
        object : ActivityResultRegistry() {
            override fun <I, O> onLaunch(
                requestCode: Int,
                contract: ActivityResultContract<I, O>,
                input: I,
                options: ActivityOptionsCompat?,
            ) {
                dispatchResult(requestCode, result)
            }
        }
}
