package org.bitcoinppl.cove.flows.cloudbackup

import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.filters.LargeTest
import androidx.test.platform.app.InstrumentationRegistry
import androidx.test.uiautomator.By
import androidx.test.uiautomator.UiDevice
import androidx.test.uiautomator.UiObject2
import androidx.test.uiautomator.Until
import org.bitcoinppl.cove.AppManager
import org.bitcoinppl.cove.cloudbackup.CloudBackupManager
import org.bitcoinppl.cove.test.FullLaunchOnboardingRobot
import org.bitcoinppl.cove.test.FullLaunchTestRule
import org.bitcoinppl.cove.test.ManualFullLaunchTest
import org.bitcoinppl.cove.test.StagedProcessFullLaunchTest
import org.bitcoinppl.cove.test.fullLaunchDevice
import org.bitcoinppl.cove.test.launchFullApp
import org.bitcoinppl.cove.testconfig.ScriptedCloudBackupFixture
import org.bitcoinppl.cove.testconfig.ScriptedCloudStorageAccess
import org.bitcoinppl.cove.testconfig.ScriptedPasskeyProvider
import org.bitcoinppl.cove.testconfig.ScriptedPasskeyProvider.Invocation
import org.bitcoinppl.cove_core.BootstrapStep
import org.bitcoinppl.cove_core.CloudBackupRestoreAllState
import org.bitcoinppl.cove_core.CloudBackupVerificationState
import org.bitcoinppl.cove_core.Route
import org.bitcoinppl.cove_core.SettingsRoute
import org.bitcoinppl.cove_core.bootstrapProgress
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import java.io.ByteArrayOutputStream

@ManualFullLaunchTest
@LargeTest
@RunWith(AndroidJUnit4::class)
class CloudBackupRestoreAllProcessDeathFullLaunchTest {
    private companion object {
        const val AUTOMATIC_RESTORE_SETTLE_MS = 1_000L
        const val EXPECTED_REMAINING_WALLETS = 2u
    }

    private lateinit var device: UiDevice

    @get:Rule
    val fullLaunch = FullLaunchTestRule()

    @Before
    fun prepareDevice() {
        ScriptedPasskeyProvider.reset()
        device = fullLaunchDevice()
    }

    @Test
    @StagedProcessFullLaunchTest
    fun scU09ProcessStage1NavigationKeepsSameRunningBatchAndLeavesDurableMarker() {
        ScriptedCloudStorageAccess.configureProductionFixtureRestore()
        assertTrue(
            "expected the first process in the SC-U09 relaunch scenario",
            ScriptedCloudStorageAccess.recordFixtureProcessAndCheckRestart(
                resetStoredState = true,
                expectedRestart = false,
            ),
        )
        launchFullApp()

        restoreInitialFixtureAndFinishOnboarding()
        ScriptedCloudStorageAccess.exposeAllProductionFixtureWallets()
        openCloudBackup()
        waitUntil("expected two authoritative cloud-only wallets before Restore All") {
            CloudBackupManager.getInstance().restoreAllState ==
                CloudBackupRestoreAllState.StartAvailable(EXPECTED_REMAINING_WALLETS)
        }

        ScriptedCloudStorageAccess.blockNextWalletDownload(
            recordId = ScriptedCloudBackupFixture.WALLET_TWO_RECORD_ID,
            matchingRequestsToSkip = 1,
        )
        device.findTextByScrolling("Restore All (2)").clickClickableAncestor()

        assertTrue(
            "expected SC-U09 Restore All to remain inside its first atomic wallet",
            ScriptedCloudStorageAccess.awaitBlockedWalletDownload(),
        )
        val runningBeforeNavigation = requireRunningRestoreAll()
        assertEquals(0u, runningBeforeNavigation.completed)
        assertEquals(EXPECTED_REMAINING_WALLETS, runningBeforeNavigation.total)
        device.findTextByScrolling("0 of 2 complete")

        openSettings()
        assertSameInFlightBatch(runningBeforeNavigation, requireRunningRestoreAll())

        openCloudBackup()
        device.findTextByScrolling("0 of 2 complete")
        assertSameInFlightBatch(runningBeforeNavigation, requireRunningRestoreAll())
    }

    @Test
    @StagedProcessFullLaunchTest
    fun scU09ProcessStage2FreshProcessShowsAuthoritativeRetryWithoutAutomaticWork() {
        ScriptedCloudStorageAccess.configureProductionFixtureRestore()
        ScriptedCloudStorageAccess.exposeAllProductionFixtureWallets()
        assertTrue(
            "expected SC-U09 stage two to run in a genuinely fresh application process",
            ScriptedCloudStorageAccess.recordFixtureProcessAndCheckRestart(
                resetStoredState = false,
                expectedRestart = true,
            ),
        )
        launchFullApp(resetData = false)

        waitUntil("expected application bootstrap to complete after process death") {
            bootstrapProgress() == BootstrapStep.COMPLETE
        }
        openCloudBackup()
        waitUntil("expected authoritative Retry Remaining after process death") {
            CloudBackupManager.getInstance().isDetailInventoryComplete &&
                CloudBackupManager.getInstance().restoreAllState ==
                CloudBackupRestoreAllState.RetryAvailable(EXPECTED_REMAINING_WALLETS)
        }
        assertTrue(
            "Retry Remaining must follow an authoritative provider inventory",
            ScriptedCloudStorageAccess.walletListCount() > 0,
        )
        device.findTextByScrolling("Retry Remaining (2)")
        val remainingWalletDownloadsAfterReconciliation = remainingWalletDownloadRecordIds()

        Thread.sleep(AUTOMATIC_RESTORE_SETTLE_MS)

        assertEquals(
            "process reconciliation must not automatically reopen passkey discovery",
            0,
            ScriptedPasskeyProvider.callCount(Invocation.DISCOVER),
        )
        assertEquals(
            "process reconciliation must not automatically present passkey authentication",
            0,
            ScriptedPasskeyProvider.callCount(Invocation.AUTHENTICATE),
        )
        assertEquals(
            "process reconciliation must not create a replacement passkey",
            0,
            ScriptedPasskeyProvider.callCount(Invocation.CREATE),
        )
        assertEquals(
            "process reconciliation must not automatically resume wallet restore work",
            remainingWalletDownloadsAfterReconciliation,
            remainingWalletDownloadRecordIds(),
        )
        assertEquals(
            CloudBackupRestoreAllState.RetryAvailable(EXPECTED_REMAINING_WALLETS),
            CloudBackupManager.getInstance().restoreAllState,
        )
    }

    private fun restoreInitialFixtureAndFinishOnboarding() {
        device.clickClickableAncestor(device.waitForText("Restore with Passkey"))
        device.waitForText("You're all set")
        device.clickClickableAncestor(device.waitForText("Done"))
        FullLaunchOnboardingRobot(device).acceptTermsAfterImport()
        assertTrue(
            "accepting terms should complete onboarding before opening Cloud Backup settings",
            device.wait(Until.gone(By.text("Terms & Conditions")), 10_000L),
        )
    }

    private fun openSettings() {
        resetRoute(Route.Settings(SettingsRoute.Main))
        device.waitForText("Security")
    }

    private fun openCloudBackup() {
        resetRoute(Route.Settings(SettingsRoute.CloudBackup))
        device.waitForText("Cloud Backup")
        waitUntil(
            message = "expected automatic verification to finish before showing recovery controls",
            timeoutMs = 60_000L,
        ) {
            val state = CloudBackupManager.getInstance().verificationState
            state != null && state !is CloudBackupVerificationState.Running
        }
    }

    private fun resetRoute(route: Route) {
        InstrumentationRegistry.getInstrumentation().runOnMainSync {
            AppManager.getInstance().resetRoute(route)
        }
    }

    private fun requireRunningRestoreAll(): CloudBackupRestoreAllState.Running {
        val state = CloudBackupManager.getInstance().restoreAllState
        assertTrue("expected Restore All to remain running, found $state", state is CloudBackupRestoreAllState.Running)

        return state as CloudBackupRestoreAllState.Running
    }

    private fun remainingWalletDownloadRecordIds(): List<String> =
        ScriptedCloudStorageAccess.walletDownloadRecordIds().filter { recordId ->
            recordId == ScriptedCloudBackupFixture.WALLET_TWO_RECORD_ID ||
                recordId == ScriptedCloudBackupFixture.WALLET_THREE_RECORD_ID
        }

    private fun assertSameInFlightBatch(
        beforeNavigation: CloudBackupRestoreAllState.Running,
        afterNavigation: CloudBackupRestoreAllState.Running,
    ) {
        assertEquals(beforeNavigation.completed, afterNavigation.completed)
        assertEquals(beforeNavigation.total, afterNavigation.total)
        assertEquals(beforeNavigation.cancellationRequested, afterNavigation.cancellationRequested)
        assertTrue(
            "navigation may project the next current wallet but must not replace the batch",
            afterNavigation.currentWalletName == null ||
                afterNavigation.currentWalletName == "Fixture Wallet Two",
        )
    }

    private fun UiDevice.waitForText(
        value: String,
        timeoutMs: Long = 20_000L,
    ): UiObject2 =
        wait(Until.findObject(By.text(value)), timeoutMs)
            ?: error("Timed out waiting for text \"$value\"")

    private fun UiDevice.findTextByScrolling(
        value: String,
        timeoutMs: Long = 20_000L,
    ): UiObject2 {
        val deadline = System.currentTimeMillis() + timeoutMs

        while (System.currentTimeMillis() < deadline) {
            findObject(By.text(value))?.let { return it }
            swipe(displayWidth / 2, displayHeight * 3 / 4, displayWidth / 2, displayHeight / 4, 20)
            Thread.sleep(250)
        }

        error("Timed out scrolling to text \"$value\"\n${windowHierarchy()}")
    }

    private fun UiDevice.windowHierarchy(): String =
        ByteArrayOutputStream().use { output ->
            dumpWindowHierarchy(output)
            output.toString()
        }

    private fun UiDevice.clickClickableAncestor(node: UiObject2) {
        node.clickClickableAncestor()
    }

    private fun UiObject2.clickClickableAncestor() {
        var clickable: UiObject2? = this

        while (clickable != null && !clickable.isClickable) {
            clickable = clickable.parent
        }

        requireNotNull(clickable) { "expected a clickable ancestor for $text" }.click()
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

        assertTrue(message, condition())
    }
}
