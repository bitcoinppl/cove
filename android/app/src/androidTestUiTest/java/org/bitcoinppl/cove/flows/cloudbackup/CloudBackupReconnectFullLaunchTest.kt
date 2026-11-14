package org.bitcoinppl.cove.flows.cloudbackup

import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.filters.LargeTest
import androidx.test.platform.app.InstrumentationRegistry
import androidx.test.uiautomator.By
import androidx.test.uiautomator.UiDevice
import androidx.test.uiautomator.Until
import org.bitcoinppl.cove.AppManager
import org.bitcoinppl.cove.cloudbackup.CloudBackupManager
import org.bitcoinppl.cove.test.FullLaunchOnboardingRobot
import org.bitcoinppl.cove.test.FullLaunchTestRule
import org.bitcoinppl.cove.test.ManualFullLaunchTest
import org.bitcoinppl.cove.test.fullLaunchDevice
import org.bitcoinppl.cove.test.launchFullApp
import org.bitcoinppl.cove.testconfig.ScriptedCloudBackupFixture
import org.bitcoinppl.cove.testconfig.ScriptedCloudStorageAccess
import org.bitcoinppl.cove.testconfig.ScriptedConnectivityAccess
import org.bitcoinppl.cove.testconfig.ScriptedPasskeyProvider
import org.bitcoinppl.cove.testconfig.ScriptedPasskeyProvider.Invocation
import org.bitcoinppl.cove_core.CloudBackupRestoreAllState
import org.bitcoinppl.cove_core.CloudBackupScreenAction
import org.bitcoinppl.cove_core.CloudBackupVerificationState
import org.bitcoinppl.cove_core.CloudOnlyOperation
import org.bitcoinppl.cove_core.Route
import org.bitcoinppl.cove_core.SettingsRoute
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

@ManualFullLaunchTest
@LargeTest
@RunWith(AndroidJUnit4::class)
class CloudBackupReconnectFullLaunchTest {
    private companion object {
        const val OFFLINE_DETAIL_MESSAGE =
            "This device is offline. Connect to the internet, then check Google Drive again."
        const val RESTORE_ALL_TITLE = "Restore All (2)"
        const val RESTORE_ACTION = "Restore to this device"
        const val DELETE_ACTION = "Delete from Cloud Backup"
        val CLOUD_ONLY_WALLET_NAMES = listOf("Fixture Wallet Two", "Fixture Wallet Three")
    }

    private lateinit var device: UiDevice

    @get:Rule
    val fullLaunch = FullLaunchTestRule()

    @Before
    fun prepareDevice() {
        ScriptedPasskeyProvider.reset()
        ScriptedConnectivityAccess.reset()
        device = fullLaunchDevice()
    }

    @After
    fun restoreConnectivity() {
        ScriptedConnectivityAccess.reset()
    }

    @Test
    fun offlineInventoryKeepsKnownRowsAndDisablesTheirActions() {
        restoreFixtureAndOpenCloudBackupDetail()
        val expectedRecordIds = cloudOnlyRecordIds()
        val passkeyCallsBeforeOffline = passkeyCallCounts()

        ScriptedConnectivityAccess.setConnected(false)
        device.openSettings()
        device.openCloudBackup()

        waitUntil("reopening the detail offline should fail the refresh and keep the known rows") {
            val manager = CloudBackupManager.getInstance()

            manager.isInventoryOffline &&
                manager.restoreAllState == CloudBackupRestoreAllState.StartDisabled(2u) &&
                cloudOnlyRecordIds() == expectedRecordIds
        }
        device.scrollUntilShown(UiTarget.Text(OFFLINE_DETAIL_MESSAGE))
        device.scrollUntilShown(UiTarget.Text(RESTORE_ALL_TITLE))
        assertFalse(
            "Restore All must stay disabled offline",
            device.waitForControl(UiTarget.Text(RESTORE_ALL_TITLE)).enabled,
        )

        // a disabled Compose row stays in the tree and reports isEnabled false, and a tap on it
        // must open neither action
        val walletDownloadsOffline = ScriptedCloudStorageAccess.walletDownloadCount()
        for (name in CLOUD_ONLY_WALLET_NAMES) {
            val row = UiTarget.Text(name)
            device.scrollUntilShown(row)
            assertFalse("$name must be disabled offline", device.waitForControl(row).enabled)

            device.tap(row)
            assertFalse(
                "$name must not open Restore or Delete offline",
                device.wait(Until.hasObject(By.text(RESTORE_ACTION)), 2_000L) ||
                    device.hasObject(By.text(DELETE_ACTION)),
            )
        }

        val manager = CloudBackupManager.getInstance()
        assertEquals("the known rows must stay listed offline", expectedRecordIds, cloudOnlyRecordIds())
        assertEquals(CloudOnlyOperation.Idle, manager.cloudOnlyOperation)
        assertEquals(walletDownloadsOffline, ScriptedCloudStorageAccess.walletDownloadCount())
        assertEquals(passkeyCallsBeforeOffline, passkeyCallCounts())
        assertEquals(1, AppManager.getInstance().wallets.size)
    }

    @Test
    fun reconnectAloneCompletesInventoryWithoutPasskeyPrompt() {
        restoreFixtureAndOpenCloudBackupDetail()
        val expectedRecordIds = cloudOnlyRecordIds()
        val passkeyCallsBeforeOffline = passkeyCallCounts()

        // a user who reopens Cloud Backup while offline gets the failed refresh
        ScriptedConnectivityAccess.setConnected(false)
        device.openSettings()
        device.openCloudBackup()

        waitUntil("reopening the detail offline should fail the refresh and keep the known rows") {
            val manager = CloudBackupManager.getInstance()

            manager.isInventoryOffline &&
                manager.restoreAllState == CloudBackupRestoreAllState.StartDisabled(2u) &&
                cloudOnlyRecordIds() == expectedRecordIds
        }
        device.scrollUntilShown(UiTarget.Text(OFFLINE_DETAIL_MESSAGE))
        device.scrollUntilShown(UiTarget.Text("Fixture Wallet Two"))
        device.scrollUntilShown(UiTarget.Text("Fixture Wallet Three"))
        device.scrollUntilShown(UiTarget.Text(RESTORE_ALL_TITLE))
        assertFalse(
            "Restore All must stay disabled while the inventory is incomplete",
            device.waitForControl(UiTarget.Text(RESTORE_ALL_TITLE)).enabled,
        )

        // the reconnect itself must refresh the open detail, so the test starts no refresh; the
        // provider counts only listings it served, so a later count shows the inventory was read
        // again rather than the offline snapshot marked complete
        val walletListsBeforeReconnect = ScriptedCloudStorageAccess.walletListCount()
        ScriptedConnectivityAccess.setConnected(true)

        waitUntil("reconnecting should complete the inventory without another refresh request") {
            val manager = CloudBackupManager.getInstance()

            ScriptedCloudStorageAccess.walletListCount() > walletListsBeforeReconnect &&
                manager.isInventoryComplete &&
                manager.restoreAllState == CloudBackupRestoreAllState.StartAvailable(2u) &&
                cloudOnlyRecordIds() == expectedRecordIds
        }
        assertTrue(
            "the offline message should clear once the inventory completes",
            device.wait(Until.gone(By.text(OFFLINE_DETAIL_MESSAGE)), 10_000L),
        )
        device.scrollUntilShown(UiTarget.Text("Fixture Wallet Two"))
        device.scrollUntilShown(UiTarget.Text("Fixture Wallet Three"))
        device.scrollUntilShown(UiTarget.Text(RESTORE_ALL_TITLE))
        assertTrue(
            "Restore All should be available again after reconnecting",
            device.waitForControl(UiTarget.Text(RESTORE_ALL_TITLE)).enabled,
        )
        assertEquals(
            "refreshing the inventory must neither create nor ask for a passkey",
            passkeyCallsBeforeOffline,
            passkeyCallCounts(),
        )
        assertEquals(0, ScriptedPasskeyProvider.callCount(Invocation.CREATE))
    }

    private fun restoreFixtureAndOpenCloudBackupDetail() {
        ScriptedCloudStorageAccess.configureProductionFixtureRestore()
        launchFullApp()

        device.waitForText("Google Drive Backup Found")
        device.tap(UiTarget.Text("Restore with Passkey"))
        device.waitForText("You're all set")
        device.waitForText("Your wallets have been restored.")
        ScriptedCloudStorageAccess.exposeAllProductionFixtureWallets()
        device.tap(UiTarget.Text("Done"))
        FullLaunchOnboardingRobot(device).acceptTermsAfterImport()
        assertTrue(
            "accepting terms should complete onboarding before opening Cloud Backup settings",
            device.wait(Until.gone(By.text("Terms & Conditions")), 10_000L),
        )

        // onboarding's last step selects the restored wallet and resets the route, which would
        // replace a settings route opened before it
        waitUntil("onboarding should finish on the restored wallet") {
            val app = AppManager.getInstance()
            val walletId = app.wallets.singleOrNull()?.id

            walletId != null &&
                app.router.default == Route.SelectedWallet(walletId) &&
                app.router.routes.isEmpty()
        }

        device.openSettings()
        val walletListsBeforeEntry = ScriptedCloudStorageAccess.walletListCount()
        device.openCloudBackup()

        // the detail held from the restore can already look complete, so wait for the refresh the
        // entry starts and the restore's upload confirmation; a restore doesn't count as
        // verification and nothing verifies on its own, so the backup stays not verified
        waitUntil(
            "expected the entry refresh to settle with two fixture wallets and Restore All",
            timeoutMs = 60_000L,
        ) {
            val manager = CloudBackupManager.getInstance()

            ScriptedCloudStorageAccess.walletListCount() > walletListsBeforeEntry &&
                manager.isInventoryComplete &&
                !manager.isInventoryChecking &&
                !manager.hasPendingUploadVerification &&
                manager.verificationState == CloudBackupVerificationState.NotVerified &&
                manager.restoreAllState == CloudBackupRestoreAllState.StartAvailable(2u) &&
                cloudOnlyRecordIds() ==
                listOf(
                    ScriptedCloudBackupFixture.WALLET_TWO_RECORD_ID,
                    ScriptedCloudBackupFixture.WALLET_THREE_RECORD_ID,
                )
        }
        CloudBackupManager.getInstance().assertNextActions(
            "a restored backup should offer Verify",
            available = setOf(CloudBackupScreenAction.VERIFY),
        )
    }

    private fun passkeyCallCounts(): Map<Invocation, Int> =
        Invocation.entries.associateWith(ScriptedPasskeyProvider::callCount)

    private fun cloudOnlyRecordIds(): List<String> =
        CloudBackupManager.getInstance().notOnDeviceRows.map { row -> row.recordId }

    private fun UiDevice.openSettings() {
        InstrumentationRegistry.getInstrumentation().runOnMainSync {
            AppManager.getInstance().resetRoute(Route.Settings(SettingsRoute.Main))
        }

        waitForText("Security")
    }

    private fun UiDevice.openCloudBackup() {
        InstrumentationRegistry.getInstrumentation().runOnMainSync {
            AppManager.getInstance().resetRoute(Route.Settings(SettingsRoute.CloudBackup))
        }

        waitForText("Cloud Backup")
    }

    private fun UiDevice.waitForText(
        value: String,
        timeoutMs: Long = 20_000L,
    ) {
        if (!wait(Until.hasObject(By.text(value)), timeoutMs)) error("Timed out waiting for text \"$value\"")
    }

    private fun waitUntil(
        message: String,
        timeoutMs: Long = 30_000L,
        condition: () -> Boolean,
    ) {
        val deadline = System.currentTimeMillis() + timeoutMs

        while (System.currentTimeMillis() < deadline) {
            if (condition()) return

            Thread.sleep(100)
        }

        val manager = CloudBackupManager.getInstance()
        val diagnostics =
            "screen=${manager.describeScreen()}, " +
                "restoreAll=${manager.restoreAllState}, cloudOnly=${cloudOnlyRecordIds()}, " +
                "passkeyCalls=${passkeyCallCounts()}, walletLists=${ScriptedCloudStorageAccess.walletListCount()}"
        assertTrue("$message; $diagnostics", condition())
    }
}
