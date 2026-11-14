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
import org.bitcoinppl.cove.test.QuarantinedInCi
import org.bitcoinppl.cove.test.fullLaunchDevice
import org.bitcoinppl.cove.test.launchFullApp
import org.bitcoinppl.cove.testconfig.ScriptedCloudBackupFixture
import org.bitcoinppl.cove.testconfig.ScriptedCloudStorageAccess
import org.bitcoinppl.cove.testconfig.ScriptedCloudStorageAccess.FixtureWalletDownloadResult
import org.bitcoinppl.cove.testconfig.ScriptedPasskeyProvider
import org.bitcoinppl.cove_core.CloudBackupListedRecord
import org.bitcoinppl.cove_core.CloudBackupRestoreAllState
import org.bitcoinppl.cove_core.Route
import org.bitcoinppl.cove_core.SettingsRoute
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

@ManualFullLaunchTest
@LargeTest
@RunWith(AndroidJUnit4::class)
class CloudBackupRestoreAllScenarioFullLaunchTest {
    private lateinit var device: UiDevice

    @get:Rule
    val fullLaunch = FullLaunchTestRule()

    @Before
    fun prepareDevice() {
        ScriptedPasskeyProvider.reset()
        device = fullLaunchDevice()
    }

    @Test
    @QuarantinedInCi(
        reason = "flaky on the emulator: failed in 8 of 12 local reruns, cause not yet diagnosed",
        issue = "https://github.com/bitcoinppl/cove/issues/1102",
        expires = "2026-12-09",
    )
    fun mixedResultMovesSuccessfulRowAndSupportsRowLocalRetry() {
        restoreFixtureAndOpenCloudBackupDetail()

        ScriptedCloudStorageAccess.blockNextWalletDownload(
            recordId = ScriptedCloudBackupFixture.WALLET_TWO_RECORD_ID,
            matchingRequestsToSkip = 1,
        )

        try {
            device.scrollAndTap(UiTarget.Text("Restore All (2)"))
            assertTrue(
                "expected a provider read after authoritative batch preparation",
                ScriptedCloudStorageAccess.awaitBlockedWalletDownload(),
            )
            val running = CloudBackupManager.getInstance().restoreAllState
            assertTrue("expected Restore All to remain running, found $running", running is CloudBackupRestoreAllState.Running)
            running as CloudBackupRestoreAllState.Running
            assertEquals(0u, running.completed)
            assertEquals(2u, running.total)
            ScriptedCloudStorageAccess.configureFixtureWalletDownloadDefault(
                ScriptedCloudBackupFixture.WALLET_THREE_RECORD_ID,
                FixtureWalletDownloadResult.CORRUPT,
            )
        } finally {
            ScriptedCloudStorageAccess.releaseBlockedWalletDownload()
        }

        waitUntil("mixed Restore All should retain only the failed wallet") {
            val manager = CloudBackupManager.getInstance()
            val failedWallet =
                (manager.notOnDeviceRows.singleOrNull()?.record as? CloudBackupListedRecord.NotOnDevice)?.wallet

            manager.restoreAllState is CloudBackupRestoreAllState.RetryAvailable &&
                failedWallet?.recordId == ScriptedCloudBackupFixture.WALLET_THREE_RECORD_ID &&
                failedWallet.restoreFailure != null &&
                localWalletNames().contains("Fixture Wallet Two") &&
                !localWalletNames().contains("Fixture Wallet Three")
        }
        assertEquals(
            "the successful row should move out of the cloud-only section",
            listOf(ScriptedCloudBackupFixture.WALLET_THREE_RECORD_ID),
            cloudOnlyRecordIds(),
        )
        device.scrollUntilShown(
            UiTarget.Text("Retry Remaining (1)"),
            message = "wallet-local failure should expose the authoritative remaining batch",
        )
        ScriptedCloudStorageAccess.configureFixtureWalletDownloadDefault(
            ScriptedCloudBackupFixture.WALLET_THREE_RECORD_ID,
            FixtureWalletDownloadResult.VALID,
        )
        device.scrollAndTap(UiTarget.Text("Fixture Wallet Three"), upwardFirst = true)
        device.scrollAndTap(UiTarget.Text("Retry restore"))

        waitUntil("row-local retry should restore the remaining wallet") {
            cloudOnlyRecordIds().isEmpty() &&
                CloudBackupManager.getInstance().restoreAllState is CloudBackupRestoreAllState.NotShown &&
                localWalletNames().contains("Fixture Wallet Three")
        }
        assertTrue(
            "row-local retry should remove Retry Remaining",
            device.wait(Until.gone(By.text("Retry Remaining (1)")), 10_000L),
        )
    }

    @Test
    @QuarantinedInCi(
        reason = "flaky on the emulator: failed in 8 of 12 local reruns, cause not yet diagnosed",
        issue = "https://github.com/bitcoinppl/cove/issues/1103",
        expires = "2026-12-09",
    )
    fun cancellationFinishesCurrentWalletAndSkipsTheNextWallet() {
        restoreFixtureAndOpenCloudBackupDetail()

        ScriptedCloudStorageAccess.blockNextWalletDownload(
            recordId = ScriptedCloudBackupFixture.WALLET_TWO_RECORD_ID,
            matchingRequestsToSkip = 1,
        )

        try {
            device.scrollAndTap(UiTarget.Text("Restore All (2)"))
            assertTrue(
                "expected Restore All to block while restoring the first wallet",
                ScriptedCloudStorageAccess.awaitBlockedWalletDownload(),
            )

            device.scrollAndTap(UiTarget.Text("Cancel"))
            waitUntil("cancellation should be requested without interrupting the active wallet") {
                val state = CloudBackupManager.getInstance().restoreAllState
                state is CloudBackupRestoreAllState.Running && state.cancellationRequested
            }
        } finally {
            ScriptedCloudStorageAccess.releaseBlockedWalletDownload()
        }

        waitUntil("cancellation should settle at the next wallet boundary") {
            CloudBackupManager.getInstance().restoreAllState is CloudBackupRestoreAllState.RetryAvailable &&
                cloudOnlyRecordIds() == listOf(ScriptedCloudBackupFixture.WALLET_THREE_RECORD_ID) &&
                localWalletNames().contains("Fixture Wallet Two") &&
                !localWalletNames().contains("Fixture Wallet Three")
        }
        assertTrue(
            "one remaining wallet should not expose Restore All after clean cancellation",
            device.findObject(By.text("Restore All (1)")) == null,
        )
        device.scrollUntilShown(
            UiTarget.Text("Retry Remaining (1)"),
            message = "cancellation should expose the authoritative remaining batch",
        )
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

        device.openSettings()
        device.openCloudBackup()
        waitUntil("expected two fixture wallets and Restore All to become available") {
            CloudBackupManager.getInstance().isInventoryComplete &&
                CloudBackupManager.getInstance().restoreAllState is CloudBackupRestoreAllState.StartAvailable &&
                cloudOnlyRecordIds() ==
                listOf(
                    ScriptedCloudBackupFixture.WALLET_TWO_RECORD_ID,
                    ScriptedCloudBackupFixture.WALLET_THREE_RECORD_ID,
                )
        }
    }

    private fun cloudOnlyRecordIds(): List<String> =
        CloudBackupManager.getInstance().notOnDeviceRows.map { row -> row.recordId }

    private fun localWalletNames(): Set<String> =
        AppManager.getInstance().wallets.mapTo(mutableSetOf()) { wallet -> wallet.name }

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
            "screen=${manager.describeScreen()}, cloudOnly=${cloudOnlyRecordIds()}, " +
                "enabled=${manager.isCloudBackupEnabled}, localWallets=${AppManager.getInstance().wallets}, " +
                "walletLists=${ScriptedCloudStorageAccess.walletListNamespaces()}, " +
                "walletDownloads=${ScriptedCloudStorageAccess.walletDownloadRecordIds()}"
        assertTrue("$message; $diagnostics", condition())
    }
}
