package org.bitcoinppl.cove.flows.cloudbackup

import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.filters.LargeTest
import androidx.test.platform.app.InstrumentationRegistry
import androidx.test.uiautomator.By
import androidx.test.uiautomator.UiDevice
import androidx.test.uiautomator.Until
import kotlinx.coroutines.runBlocking
import org.bitcoinppl.cove.AppManager
import org.bitcoinppl.cove.AuthManager
import org.bitcoinppl.cove.TaggedItem
import org.bitcoinppl.cove.UnlockMode
import org.bitcoinppl.cove.test.FullLaunchTestRule
import org.bitcoinppl.cove.test.ManualFullLaunchTest
import org.bitcoinppl.cove.test.fullLaunchDevice
import org.bitcoinppl.cove.test.hierarchy
import org.bitcoinppl.cove.test.restoreNoAppLock
import org.bitcoinppl.cove.testconfig.ScriptedPasskeyProvider
import org.bitcoinppl.cove_core.AppAlertState
import org.bitcoinppl.cove_core.types.WalletId
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Before
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

/**
 * The missing-key alert offers Open Cloud Backup in main mode only
 *
 * Decoy mode hides Cloud Backup, so the button would lead from the decoy wallets to the main
 * wallets' backup. The alert is raised in process, as the wallet manager raises it when a hot
 * wallet's key is gone, since the rule under test is which buttons it draws
 */
@ManualFullLaunchTest
@LargeTest
@RunWith(AndroidJUnit4::class)
class DecoyMissingKeyAlertFullLaunchTest {
    private lateinit var device: UiDevice
    private lateinit var robot: CloudBackupTurnOffRobot

    @get:Rule
    val fullLaunch = FullLaunchTestRule()

    @Before
    fun prepareDevice() {
        ScriptedPasskeyProvider.reset()
        device = fullLaunchDevice()
        robot = CloudBackupTurnOffRobot(device)
    }

    @After
    fun cleanUp() {
        onMain {
            val auth = AuthManager.getInstance()
            if (auth.isInDecoyMode()) auth.switchToMainMode()
            AppManager.getInstance().alertState = null
        }
        restoreNoAppLock()
    }

    @Test
    fun decoyModeMissingKeyAlertOffersNoCloudBackup() {
        robot.freshEnableAndOpenDetail()
        val walletId = AppManager.getInstance().wallets.single().id

        onMain {
            val auth = AuthManager.getInstance()
            auth.setPin(MAIN_PIN)
            auth.setDecoyPin(DECOY_PIN)
        }

        // main mode offers it, which shows Cloud Backup is on for the decoy check below
        showMissingKeyAlert(walletId)
        robot.waitUntil("main mode's missing-key alert didn't offer Open Cloud Backup") {
            device.hasObject(By.text(OPEN_CLOUD_BACKUP))
        }
        dismissAlert()

        var mode: UnlockMode? = null
        onMain { mode = runBlocking { AuthManager.getInstance().handleAndReturnUnlockMode(DECOY_PIN) } }
        assertEquals("the decoy PIN didn't unlock decoy mode", UnlockMode.DECOY, mode)

        showMissingKeyAlert(walletId)
        assertFalse(
            "decoy mode's missing-key alert offered Open Cloud Backup\n${device.hierarchy()}",
            device.hasObject(By.text(OPEN_CLOUD_BACKUP)),
        )
    }

    private fun showMissingKeyAlert(walletId: WalletId) {
        onMain { AppManager.getInstance().alertState = TaggedItem(AppAlertState.HotWalletKeyMissing(walletId)) }

        if (!device.wait(Until.hasObject(By.text(ALERT_TITLE)), ALERT_TIMEOUT_MS)) {
            error("the missing-key alert never showed\n${device.hierarchy()}")
        }
    }

    private fun dismissAlert() {
        onMain { AppManager.getInstance().alertState = null }

        if (!device.wait(Until.gone(By.text(ALERT_TITLE)), ALERT_TIMEOUT_MS)) {
            error("the missing-key alert never closed\n${device.hierarchy()}")
        }
    }

    private fun onMain(block: () -> Unit) {
        InstrumentationRegistry.getInstrumentation().runOnMainSync(block)
    }

    private companion object {
        const val MAIN_PIN = "135791"
        const val DECOY_PIN = "246802"
        const val ALERT_TITLE = "Wallet Needs Recovery"
        const val OPEN_CLOUD_BACKUP = "Open Cloud Backup"
        const val ALERT_TIMEOUT_MS = 10_000L
    }
}
