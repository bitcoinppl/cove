package org.bitcoinppl.cove.flows.cloudbackup

import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.filters.LargeTest
import androidx.test.platform.app.InstrumentationRegistry
import androidx.test.uiautomator.By
import androidx.test.uiautomator.UiDevice
import androidx.test.uiautomator.UiObject2
import androidx.test.uiautomator.Until
import org.bitcoinppl.cove.AppManager
import org.bitcoinppl.cove.AuthManager
import org.bitcoinppl.cove.test.FullLaunchTestRule
import org.bitcoinppl.cove.test.ManualFullLaunchTest
import org.bitcoinppl.cove.test.fullLaunchDevice
import org.bitcoinppl.cove.test.restoreNoAppLock
import org.bitcoinppl.cove.testconfig.ScriptedCloudStorageAccess
import org.bitcoinppl.cove.testconfig.ScriptedPasskeyProvider
import org.bitcoinppl.cove_core.AuthManagerAction
import org.bitcoinppl.cove_core.WalletType
import org.junit.After
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.FixMethodOrder
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.junit.runners.MethodSorters
import java.util.regex.Pattern

/**
 * A new hot wallet backed up by Cloud Backup needs no word verification before the Wipe Data PIN
 * can be turned on, both right after enabling Cloud Backup and after a relaunch
 *
 * The scenario runs as two ordered stages: the test orchestrator gives every test method its own
 * instrumentation run, which force-stops the app, so stage two starts in a fresh process against
 * the cloud and app data stage one left. The relaunch matters because it rebuilds Cloud Backup's
 * live state from what was persisted
 */
@ManualFullLaunchTest
@LargeTest
@RunWith(AndroidJUnit4::class)
@FixMethodOrder(MethodSorters.NAME_ASCENDING)
class CloudBackupWipeDataPinFullLaunchTest {
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
    fun removeAppLock() {
        restoreNoAppLock()
    }

    @Test
    fun stage1FreshCloudBackupAllowsTheWipeDataPin() {
        robot.freshEnableAndOpenDetail(persistent = true)
        assertOnlyAnUnverifiedHotWallet()

        assertWipeDataPinOffersToEnable()
    }

    @Test
    fun stage2RelaunchedCloudBackupAllowsTheWipeDataPin() {
        ScriptedCloudStorageAccess.configurePersistentFreshEnableWithDelayedVisibility(resetStoredState = false)
        assertTrue(
            "stage two needs stage one's backup and a fresh process, so run the whole class",
            ScriptedCloudStorageAccess.recordProcessAndCheckRestart(expectedRestart = true),
        )
        robot.launchApp(resetData = false)
        assertOnlyAnUnverifiedHotWallet()

        assertWipeDataPinOffersToEnable()
    }

    // the wallet's words were never verified, so only its cloud copy can allow the Wipe Data PIN
    private fun assertOnlyAnUnverifiedHotWallet() {
        val wallet = AppManager.getInstance().wallets.single()

        assertTrue("the fresh wallet should be a hot wallet", wallet.walletType == WalletType.HOT)
        assertFalse("the fresh wallet's words should be unverified", wallet.verified)
    }

    private fun assertWipeDataPinOffersToEnable() {
        robot.openSettings()

        // the Wipe Data PIN row shows only once a PIN is set
        InstrumentationRegistry.getInstrumentation().runOnMainSync {
            AuthManager.getInstance().dispatch(AuthManagerAction.SetPin(PIN))
        }
        robot.scrollToText(WIPE_DATA_PIN_ROW)
        device.switchInRow(WIPE_DATA_PIN_ROW).click()

        // the unverified-wallets alert is checked first, so the confirmation proves it was skipped
        val anyAnswer = By.text(Pattern.compile("${Pattern.quote(CONFIRM_ENABLE)}|${Pattern.quote(UNVERIFIED_WALLETS_TITLE)}"))
        assertTrue("turning on the Wipe Data PIN showed no dialog", device.wait(Until.hasObject(anyAnswer), 10_000L))
        assertFalse(
            "Cloud Backup covers the wallet, so it must not need verifying first",
            device.hasObject(By.text(UNVERIFIED_WALLETS_TITLE)),
        )
        assertTrue("turning on the Wipe Data PIN should ask to confirm it", device.hasObject(By.text(CONFIRM_ENABLE)))

        robot.clickText(CANCEL)
    }

    // the row's switch carries no text of its own, so it is the nearest switch around the row's title
    private fun UiDevice.switchInRow(title: String): UiObject2 {
        var node: UiObject2? = requireNotNull(findObject(By.text(title))) { "no row titled \"$title\"" }

        while (node != null) {
            node.findObject(By.checkable(true))?.let { return it }
            node = node.parent
        }

        error("the row titled \"$title\" has no switch")
    }

    private companion object {
        const val PIN = "135790"
        const val WIPE_DATA_PIN_ROW = "Enable Wipe Data PIN"
        const val CONFIRM_ENABLE = "Yes, Enable Wipe Data PIN"
        const val UNVERIFIED_WALLETS_TITLE = "Can't Enable Wipe Data PIN"
        const val CANCEL = "Cancel"
    }
}
