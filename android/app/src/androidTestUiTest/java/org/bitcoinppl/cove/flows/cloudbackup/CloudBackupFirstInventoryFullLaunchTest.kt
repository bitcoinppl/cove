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
import org.bitcoinppl.cove_core.BootstrapStep
import org.bitcoinppl.cove_core.CloudBackupManagerAction
import org.bitcoinppl.cove_core.CloudBackupRestoreAllState
import org.bitcoinppl.cove_core.CloudBackupVerificationState
import org.bitcoinppl.cove_core.CloudOnlyOperation
import org.bitcoinppl.cove_core.Route
import org.bitcoinppl.cove_core.SettingsRoute
import org.bitcoinppl.cove_core.bootstrapProgress
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

/**
 * Holds a relaunched app's first cloud inventory load, every listing and the snapshot Cloud Backup
 * detail reads, and checks that nothing can restore or delete a cloud-only wallet until the
 * provider answers
 *
 * Cloud-only rows aren't kept across a relaunch and load only once the inventory is ready, so the
 * held detail must list none, and a restore or delete sent straight to the manager must do nothing.
 * A single process can't hold the first load, since finishing onboarding starts discovery before a
 * test can block the listing, so the first stage restores the fixture and the second, a fresh
 * process, holds the listings before the app starts. Run it with
 * `cargo xtask android-ui-first-inventory`
 */
@ManualFullLaunchTest
@LargeTest
@RunWith(AndroidJUnit4::class)
class CloudBackupFirstInventoryFullLaunchTest {
    private lateinit var device: UiDevice

    @get:Rule
    val fullLaunch = FullLaunchTestRule()

    @Before
    fun prepareDevice() {
        ScriptedPasskeyProvider.reset()
        device = fullLaunchDevice()
    }

    @After
    fun releaseListings() {
        // a failed assertion must not leave the app parked inside a listing
        ScriptedCloudStorageAccess.releaseWalletListGate()
        ScriptedCloudStorageAccess.releaseBlockedWalletListSnapshot()
    }

    @Test
    @StagedProcessFullLaunchTest
    fun stage1RestoreTheFixtureAndConfirmBothCloudOnlyRows() {
        ScriptedCloudStorageAccess.configureProductionFixtureRestore()
        assertTrue(
            "expected the first process of the first-inventory scenario",
            ScriptedCloudStorageAccess.recordFixtureProcessAndCheckRestart(
                resetStoredState = true,
                expectedRestart = false,
            ),
        )

        launchFullApp()
        awaitBootstrap()

        device.waitForText("Google Drive Backup Found")
        device.clickableAncestor(device.waitForText("Restore with Passkey")).click()
        device.waitForText("You're all set")
        ScriptedCloudStorageAccess.exposeAllProductionFixtureWallets()
        device.clickableAncestor(device.waitForText("Done")).click()
        FullLaunchOnboardingRobot(device).acceptTermsAfterImport()
        assertTrue(
            "accepting terms should complete onboarding before opening Cloud Backup settings",
            device.wait(Until.gone(By.text("Terms & Conditions")), 10_000L),
        )

        openCloudBackup()
        waitUntil("the provider should confirm both cloud-only rows before the relaunch") {
            val manager = CloudBackupManager.getInstance()

            manager.isInventoryComplete &&
                cloudOnlyRecordIds() == FIXTURE_CLOUD_ONLY_RECORD_IDS &&
                manager.restoreAllState == CloudBackupRestoreAllState.StartAvailable(2u)
        }
    }

    @Test
    @StagedProcessFullLaunchTest
    fun stage2RelaunchKeepsActionsDisabledUntilTheFirstListingAnswers() {
        ScriptedCloudStorageAccess.configureProductionFixtureRestore()
        ScriptedCloudStorageAccess.exposeAllProductionFixtureWallets()
        assertTrue(
            "expected the second stage to run in a fresh application process",
            ScriptedCloudStorageAccess.recordFixtureProcessAndCheckRestart(
                resetStoredState = false,
                expectedRestart = true,
            ),
        )

        // both holds are set before the activity starts, so bootstrap's discovery, sync health and
        // passkey verification all wait at the gate and the detail's snapshot is the first snapshot
        ScriptedCloudStorageAccess.closeWalletListGate()
        ScriptedCloudStorageAccess.blockNextWalletListSnapshot()
        launchFullApp(resetData = false)
        awaitBootstrap()
        assertTrue(
            "bootstrap should start inventory discovery, which the provider holds",
            ScriptedCloudStorageAccess.awaitWalletListAtGate(timeoutMs = 30_000L),
        )

        openCloudBackup()
        assertTrue(
            "opening Cloud Backup should check the inventory, which the provider holds",
            ScriptedCloudStorageAccess.awaitBlockedWalletListSnapshot(timeoutMs = 30_000L),
        )

        assertNothingCanRestoreOrDeleteWhileHeld()

        ScriptedCloudStorageAccess.releaseWalletListGate()
        ScriptedCloudStorageAccess.releaseBlockedWalletListSnapshot()

        // the gate also held passkey verification, which finishes after the release and can
        // redraw the detail, so the rows are tapped only once it has settled
        waitUntil("the provider's answer should confirm both cloud-only rows") {
            val manager = CloudBackupManager.getInstance()

            manager.isInventoryComplete &&
                !manager.isInventoryChecking &&
                manager.verificationState is CloudBackupVerificationState.Verified &&
                cloudOnlyRecordIds() == FIXTURE_CLOUD_ONLY_RECORD_IDS &&
                manager.restoreAllState == CloudBackupRestoreAllState.StartAvailable(2u)
        }

        device.waitForIdle()
        assertRowOpensEnabledActions(WALLET_TWO)
    }

    /**
     * Before the provider answers the inventory isn't ready: Restore All can't start, no cloud-only
     * row is listed, and a restore or delete sent straight to the manager does nothing
     */
    private fun assertNothingCanRestoreOrDeleteWhileHeld() {
        val manager = CloudBackupManager.getInstance()
        waitUntil("the held first listing should leave the inventory checking") {
            manager.isInventoryChecking
        }

        assertInventoryStillWaits("before the provider answers")
        assertEquals("no cloud-only row may load before the provider answers", emptyList<String>(), cloudOnlyRecordIds())
        for (name in FIXTURE_WALLET_NAMES.values) {
            assertFalse("$name must not be listed before the provider answers", device.hasObject(By.text(name)))
        }

        val localWalletsBefore = AppManager.getInstance().wallets.map { it.id }
        val downloadsBefore = ScriptedCloudStorageAccess.walletDownloadRecordIds()
        dispatchOnMain(CloudBackupManagerAction.RestoreCloudWallet(ScriptedCloudBackupFixture.WALLET_TWO_RECORD_ID))
        dispatchOnMain(CloudBackupManagerAction.DeleteCloudWallet(ScriptedCloudBackupFixture.WALLET_THREE_RECORD_ID))
        Thread.sleep(ACTION_SETTLE_MS)

        // the gate still holds every listing, so nothing confirmed the inventory while the
        // dispatched actions ran
        assertInventoryStillWaits("after the dispatched restore and delete")
        assertEquals("a restore must not start before the provider answers", CloudOnlyOperation.Idle, manager.cloudOnlyOperation)
        assertEquals(
            "a restore must not add a wallet before the provider answers",
            localWalletsBefore,
            AppManager.getInstance().wallets.map { it.id },
        )

        assertEquals(
            "a restore must not download a backup before the provider answers",
            downloadsBefore,
            ScriptedCloudStorageAccess.walletDownloadRecordIds(),
        )

        assertEquals(
            "a delete must not remove a backup before the provider answers",
            emptyList<String>(),
            ScriptedCloudStorageAccess.deletedWalletRecordIds(),
        )
    }

    private fun assertInventoryStillWaits(phase: String) {
        val manager = CloudBackupManager.getInstance()

        assertFalse("the inventory must not be complete $phase", manager.isInventoryComplete)
        assertFalse(
            "Restore All must not be available $phase",
            manager.restoreAllState is CloudBackupRestoreAllState.StartAvailable ||
                manager.restoreAllState is CloudBackupRestoreAllState.RetryAvailable,
        )
    }

    private fun assertRowOpensEnabledActions(name: String) {
        device.scrollUntilText(name)
        waitUntil("$name should be enabled once the inventory is complete") {
            device.findObject(By.text(name))?.let { device.clickableAncestor(it).isEnabled } == true
        }

        device.clickableAncestor(device.waitForText(name)).click()
        for (action in listOf(RESTORE_ACTION, DELETE_ACTION)) {
            assertTrue(
                "$action should be enabled once the inventory is complete",
                device.clickableAncestor(device.waitForText(action)).isEnabled,
            )
        }
    }

    // the activity starts before bootstrap finishes, and AppManager is unavailable until it does
    private fun awaitBootstrap() {
        val deadline = System.currentTimeMillis() + 30_000L

        while (bootstrapProgress() != BootstrapStep.COMPLETE && System.currentTimeMillis() < deadline) {
            Thread.sleep(100)
        }

        assertEquals("the app should finish bootstrapping after launch", BootstrapStep.COMPLETE, bootstrapProgress())
    }

    private fun openCloudBackup() {
        InstrumentationRegistry.getInstrumentation().runOnMainSync {
            AppManager.getInstance().resetRoute(Route.Settings(SettingsRoute.CloudBackup))
        }

        device.waitForText("Cloud Backup")
    }

    private fun dispatchOnMain(action: CloudBackupManagerAction) {
        InstrumentationRegistry.getInstrumentation().runOnMainSync {
            CloudBackupManager.getInstance().dispatch(action)
        }
    }

    private fun cloudOnlyRecordIds(): List<String> =
        CloudBackupManager.getInstance().notOnDeviceRows.map { row -> row.recordId }

    private fun UiDevice.waitForText(
        value: String,
        timeoutMs: Long = 20_000L,
    ): UiObject2 =
        wait(Until.findObject(By.text(value)), timeoutMs)
            ?: error("Timed out waiting for text \"$value\"")

    private fun UiDevice.scrollUntilText(
        value: String,
        timeoutMs: Long = 20_000L,
    ): UiObject2 {
        val deadline = System.currentTimeMillis() + timeoutMs

        while (System.currentTimeMillis() < deadline) {
            findObject(By.text(value))?.let { return it }
            swipe(displayWidth / 2, displayHeight * 3 / 4, displayWidth / 2, displayHeight / 4, 20)

            // a tap while the list still flings only stops it, so the next tap must wait for it to settle
            waitForIdle()
        }

        error("Timed out scrolling to text \"$value\"")
    }

    // a disabled Compose clickable stays clickable in the accessibility tree and reports isEnabled false
    private fun UiDevice.clickableAncestor(node: UiObject2): UiObject2 {
        var clickable: UiObject2? = node

        while (clickable != null && !clickable.isClickable) {
            clickable = clickable.parent
        }

        return requireNotNull(clickable) { "expected a clickable ancestor for ${node.text}" }
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
                "complete=${manager.isInventoryComplete}, cloudOnly=${cloudOnlyRecordIds()}, " +
                "restoreAll=${manager.restoreAllState}, verification=${manager.verificationState}, " +
                "walletLists=${ScriptedCloudStorageAccess.walletListCount()}"

        assertTrue("$message; $diagnostics", condition())
    }

    private companion object {
        const val WALLET_TWO = "Fixture Wallet Two"
        const val RESTORE_ACTION = "Restore to this device"
        const val DELETE_ACTION = "Delete from Cloud Backup"

        // long enough for a dispatched action to reach Rust and start its first provider call
        const val ACTION_SETTLE_MS = 2_000L

        val FIXTURE_CLOUD_ONLY_RECORD_IDS =
            listOf(
                ScriptedCloudBackupFixture.WALLET_TWO_RECORD_ID,
                ScriptedCloudBackupFixture.WALLET_THREE_RECORD_ID,
            )
        val FIXTURE_WALLET_NAMES =
            mapOf(
                ScriptedCloudBackupFixture.WALLET_TWO_RECORD_ID to WALLET_TWO,
                ScriptedCloudBackupFixture.WALLET_THREE_RECORD_ID to "Fixture Wallet Three",
            )
    }
}
