package org.bitcoinppl.cove

import androidx.test.platform.app.InstrumentationRegistry
import kotlinx.coroutines.runBlocking
import org.bitcoinppl.cove.test.bootstrapRustRuntimeForUiTest
import org.bitcoinppl.cove_core.AppAlertState
import org.bitcoinppl.cove_core.AppLockReason
import org.bitcoinppl.cove_core.AuthManagerAction
import org.bitcoinppl.cove_core.AuthType
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assume.assumeFalse
import org.junit.Assume.assumeTrue
import org.junit.Before
import org.junit.Test

/**
 * An app alert belongs to the wallet mode that raised it
 *
 * The decoy PIN must open a Cove with no hint of the main wallets, so an alert they raised, such
 * as a Cloud Backup failure, is gone by the time the lock opens, not when the mode change reaches
 * the app a moment later
 */
class AppAlertModeTest {
    private val instrumentation = InstrumentationRegistry.getInstrumentation()
    private var pinSet = false

    @Before
    fun setUp() {
        bootstrapRustRuntimeForUiTest()

        // the test sets and removes its own PINs, so never run it over a credential someone relies on
        assumeTrue("an app lock is already set up on this device", Auth.type == AuthType.NONE)
        assumeFalse("the device is in decoy mode", Auth.isInDecoyMode())

        Auth.setPin(MAIN_PIN)
        Auth.setDecoyPin(DECOY_PIN)
        pinSet = true
    }

    @After
    fun tearDown() {
        if (!pinSet) return

        instrumentation.runOnMainSync {
            if (Auth.isInDecoyMode()) Auth.switchToMainMode()
            App.alertState = null
        }

        // rust writes the auth type before dispatch returns, so the lock can be cleared right away
        Auth.dispatch(AuthManagerAction.DisableDecoyPin)
        Auth.dispatch(AuthManagerAction.DisablePin)
        instrumentation.runOnMainSync { Auth.unlockIfAuthDisabled() }
    }

    @Test
    fun theDecoyPinOpensTheAppWithoutTheMainModeAlert() {
        // one main-thread turn, as the PIN pad runs it, so a mode change queued for later can't help
        instrumentation.runOnMainSync {
            App.alertState = TaggedItem(cloudBackupFailure())
            Auth.lock(AppLockReason.BACKGROUND)

            val unlock = runBlocking { Auth.handleAndReturnUnlockMode(DECOY_PIN) }
            assertEquals(UnlockMode.DECOY, unlock)
            assertNull("the decoy PIN opened the app with the main-mode alert still up", App.alertState)
        }

        // the main PIN opens the mode the app locked in, so an alert raised there stays to be read
        instrumentation.runOnMainSync { Auth.switchToMainMode() }
        instrumentation.waitForIdleSync()

        instrumentation.runOnMainSync {
            val alert = TaggedItem(cloudBackupFailure())
            App.alertState = alert
            Auth.lock(AppLockReason.BACKGROUND)

            val unlock = runBlocking { Auth.handleAndReturnUnlockMode(MAIN_PIN) }
            assertEquals(UnlockMode.MAIN, unlock)
            assertEquals("the main PIN dropped an alert raised in main mode", alert, App.alertState)
        }
    }

    private fun cloudBackupFailure(): AppAlertState = AppAlertState.General(title = "Cloud Backup", message = "Verification failed")

    private companion object {
        const val MAIN_PIN = "975310"
        const val DECOY_PIN = "864208"
    }
}
