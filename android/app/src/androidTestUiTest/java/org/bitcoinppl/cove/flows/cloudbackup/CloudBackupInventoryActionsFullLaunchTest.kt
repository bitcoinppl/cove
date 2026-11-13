package org.bitcoinppl.cove.flows.cloudbackup

import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.filters.LargeTest
import androidx.test.platform.app.InstrumentationRegistry
import androidx.test.uiautomator.By
import androidx.test.uiautomator.StaleObjectException
import androidx.test.uiautomator.UiDevice
import androidx.test.uiautomator.UiObject2
import androidx.test.uiautomator.Until
import org.bitcoinppl.cove.AppManager
import org.bitcoinppl.cove.cloudbackup.CloudBackupManager
import org.bitcoinppl.cove.test.FullLaunchOnboardingRobot
import org.bitcoinppl.cove.test.FullLaunchTestRule
import org.bitcoinppl.cove.test.ManualFullLaunchTest
import org.bitcoinppl.cove.test.deleteTestWallet
import org.bitcoinppl.cove.test.fullLaunchDevice
import org.bitcoinppl.cove.test.launchFullApp
import org.bitcoinppl.cove.testconfig.ScriptedCloudBackupFixture
import org.bitcoinppl.cove.testconfig.ScriptedCloudStorageAccess
import org.bitcoinppl.cove.testconfig.ScriptedPasskeyProvider
import org.bitcoinppl.cove_core.BootstrapStep
import org.bitcoinppl.cove_core.CloudBackupManagerAction
import org.bitcoinppl.cove_core.CloudBackupRestoreAllState
import org.bitcoinppl.cove_core.CloudOnlyOperation
import org.bitcoinppl.cove_core.Route
import org.bitcoinppl.cove_core.SettingsRoute
import org.bitcoinppl.cove_core.bootstrapProgress
import org.bitcoinppl.cove_core.device.CloudStorageException
import org.bitcoinppl.cove_core.types.WalletId
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

/**
 * Holds the provider's wallet listing while Cloud Backup checks a known cloud-only inventory again,
 * and checks that no cloud-only row can restore or delete a wallet until the provider confirms it
 *
 * The first inventory load is not covered here: finishing onboarding can start inventory discovery
 * in the background, which can confirm the inventory before a test opens Cloud Backup and holds
 * its listing
 */
@ManualFullLaunchTest
@LargeTest
@RunWith(AndroidJUnit4::class)
class CloudBackupInventoryActionsFullLaunchTest {
    private lateinit var device: UiDevice

    // wallets the app already had once this test launched it, so cleanup removes only this test's
    private var walletIdsAtLaunch: Set<WalletId>? = null

    @get:Rule
    val fullLaunch = FullLaunchTestRule()

    @Before
    fun prepareDevice() {
        ScriptedPasskeyProvider.reset()
        device = fullLaunchDevice()
    }

    @After
    fun cleanUp() {
        // a failed assertion must not leave a listing suspended or restored wallets behind
        ScriptedCloudStorageAccess.releaseBlockedWalletListSnapshot()
        val launchedWith = walletIdsAtLaunch ?: return

        AppManager
            .getInstance()
            .wallets
            .map { it.id }
            .filterNot(launchedWith::contains)
            .forEach(::deleteTestWallet)
    }

    @Test
    fun cloudOnlyRowActionsStayDisabledWhileAKnownInventoryIsCheckedAgain() {
        restoreFixtureWallets()
        openCloudBackup()
        waitForConfirmedInventory()
        assertRowOpensEnabledActions(WALLET_TWO)

        // the sheet opened above stays open through the check
        whileKnownInventoryIsChecked(trigger = { dispatchOnMain(CloudBackupManagerAction.RefreshDetail) }) {
            assertOpenSheetActionsDisabledAndInert()
        }

        for (action in listOf(RESTORE_ACTION, DELETE_ACTION)) {
            waitUntil("$action should be enabled again once the inventory is complete") {
                device.findObject(By.text(action))?.let { device.clickableAncestor(it).isEnabled } == true
            }
        }

        // tapping Delete swaps the sheet for its own confirmation, which keeps a separate readiness gate
        device.clickableAncestor(device.waitForText(DELETE_ACTION)).click()
        device.waitForText(DELETE_FOREVER)
        whileKnownInventoryIsChecked(trigger = { dispatchOnMain(CloudBackupManagerAction.RefreshDetail) }) {
            assertDeleteForeverDisabledAndInert()
        }

        waitUntil("Delete Forever should be enabled again once the inventory is complete") {
            device.findObject(By.text(DELETE_FOREVER))?.let { device.clickableAncestor(it).isEnabled } == true
        }

        device.clickableAncestor(device.waitForText(CANCEL)).click()
        assertTrue(
            "Cancel should close the delete confirmation",
            device.wait(Until.gone(By.text(DELETE_FOREVER)), 10_000L),
        )

        whileKnownInventoryIsChecked(trigger = ::tapCheckAgainUntilTheCheckStarts) {
            assertNoCloudOnlyRowOpensItsActions()
        }

        assertRowOpensEnabledActions(WALLET_TWO)
    }

    // the error banner appears as the failed refresh lands, so a tap during that layout change can
    // miss; another tap only asks for the same refresh, and a Check Again that does nothing still
    // fails every attempt
    private fun tapCheckAgainUntilTheCheckStarts() {
        repeat(CHECK_AGAIN_ATTEMPTS) {
            device.waitForIdle()
            device.clickTextScrollingUp(CHECK_AGAIN)

            // longer than the minimum interval between refreshes, which can defer the check
            if (ScriptedCloudStorageAccess.awaitBlockedWalletListSnapshot(timeoutMs = 8_000L)) return
        }
    }

    // a refresh of a confirmed inventory keeps it complete, so each check starts from a failed
    // refresh, which leaves the known rows retained for the held check to work from
    private fun whileKnownInventoryIsChecked(
        trigger: () -> Unit,
        assertions: () -> Unit,
    ) {
        failNextRefresh()
        ScriptedCloudStorageAccess.blockNextWalletListSnapshot()
        trigger()
        assertTrue(
            "the check should list wallets from the provider again",
            ScriptedCloudStorageAccess.awaitBlockedWalletListSnapshot(timeoutMs = 20_000L),
        )

        try {
            waitForCheckingInventory()
            assertions()
        } finally {
            ScriptedCloudStorageAccess.releaseBlockedWalletListSnapshot()
        }

        waitForConfirmedInventory()
    }

    // ends the next refresh with a provider error that the refresh does not retry, so it lands in
    // Failed with the known rows retained
    private fun failNextRefresh() {
        ScriptedCloudStorageAccess.blockNextWalletListSnapshot()
        dispatchOnMain(CloudBackupManagerAction.RefreshDetail)
        assertTrue(
            "the refresh should list wallets from the provider",
            ScriptedCloudStorageAccess.awaitBlockedWalletListSnapshot(timeoutMs = 20_000L),
        )

        ScriptedCloudStorageAccess.failBlockedWalletListSnapshot(
            CloudStorageException.DownloadFailed("scripted provider listing failed"),
        )

        waitUntil("the failed refresh should keep both rows and disable Restore All") {
            val manager = CloudBackupManager.getInstance()

            manager.isInventoryIncomplete &&
                cloudOnlyRecordIds() == FIXTURE_CLOUD_ONLY_RECORD_IDS &&
                manager.restoreAllState is CloudBackupRestoreAllState.StartDisabled
        }
    }

    private fun waitForCheckingInventory() {
        waitUntil("the held listing should leave the known rows checking") {
            val manager = CloudBackupManager.getInstance()

            manager.isInventoryChecking &&
                cloudOnlyRecordIds() == FIXTURE_CLOUD_ONLY_RECORD_IDS
        }
    }

    private fun waitForConfirmedInventory() {
        waitUntil("the provider should confirm both cloud-only rows") {
            CloudBackupManager.getInstance().isInventoryComplete &&
                cloudOnlyRecordIds() == FIXTURE_CLOUD_ONLY_RECORD_IDS
        }
    }

    // every fixture row must be listed, so the check cannot pass without tapping a disabled row
    private fun assertNoCloudOnlyRowOpensItsActions() {
        assertEquals(
            "both fixture rows should stay listed while the inventory is checked",
            FIXTURE_CLOUD_ONLY_RECORD_IDS,
            cloudOnlyRecordIds(),
        )

        for (name in listOf(WALLET_TWO, WALLET_THREE)) {
            device.scrollUntilText(name)
            waitUntil("$name should be disabled until the inventory is complete") {
                device.findObject(By.text(name))?.let { device.clickableAncestor(it).isEnabled } == false
            }

            device.clickableAncestor(device.waitForText(name)).click()
            assertFalse(
                "$name must not open Restore or Delete until the inventory is complete",
                device.wait(Until.hasObject(By.text(RESTORE_ACTION)), 2_000L) ||
                    device.hasObject(By.text(DELETE_ACTION)),
            )
        }
    }

    private fun assertOpenSheetActionsDisabledAndInert() {
        for (action in listOf(RESTORE_ACTION, DELETE_ACTION)) {
            waitUntil("$action in the open sheet should be disabled while checking") {
                device.findObject(By.text(action))?.let { device.clickableAncestor(it).isEnabled } == false
            }
        }

        // background verification reads backups while the check is held, so a restore is judged by
        // its outcome, a new local wallet, rather than by provider downloads
        val localWalletsBefore = AppManager.getInstance().wallets.map { it.id }
        device.clickableAncestor(device.waitForText(RESTORE_ACTION)).click()
        device.clickableAncestor(device.waitForText(DELETE_ACTION)).click()

        assertFalse(
            "a disabled Delete must not ask to delete the backup",
            device.wait(Until.hasObject(By.text("Delete $WALLET_TWO?")), 2_000L),
        )

        assertEquals(
            "a disabled Restore must not add the wallet to this device",
            localWalletsBefore,
            AppManager.getInstance().wallets.map { it.id },
        )

        assertEquals(
            "a disabled Restore must not start a restore",
            CloudOnlyOperation.Idle,
            CloudBackupManager.getInstance().cloudOnlyOperation,
        )

        assertEquals(
            "disabled actions must leave both backups listed",
            FIXTURE_CLOUD_ONLY_RECORD_IDS,
            cloudOnlyRecordIds(),
        )
    }

    private fun assertDeleteForeverDisabledAndInert() {
        waitUntil("Delete Forever should be disabled while checking") {
            device.findObject(By.text(DELETE_FOREVER))?.let { device.clickableAncestor(it).isEnabled } == false
        }

        device.clickableAncestor(device.waitForText(DELETE_FOREVER)).click()
        assertFalse(
            "a disabled Delete Forever must leave its confirmation open",
            device.wait(Until.gone(By.text("Delete $WALLET_TWO?")), 2_000L),
        )

        assertEquals(
            "a disabled Delete Forever must not start a delete",
            CloudOnlyOperation.Idle,
            CloudBackupManager.getInstance().cloudOnlyOperation,
        )

        assertEquals(
            "a disabled Delete Forever must leave both backups listed",
            FIXTURE_CLOUD_ONLY_RECORD_IDS,
            cloudOnlyRecordIds(),
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

    private fun restoreFixtureWallets() {
        ScriptedCloudStorageAccess.configureProductionFixtureRestore()

        // the reset only runs in a fresh activity, which the test orchestrator's process per test gives
        launchFullApp()
        awaitBootstrap()
        walletIdsAtLaunch = AppManager.getInstance().wallets.map { it.id }.toSet()

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

        assertEquals(
            "the fixture should restore only its first wallet",
            1,
            AppManager.getInstance().wallets.size,
        )
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

    private fun UiDevice.clickTextScrollingUp(
        value: String,
        timeoutMs: Long = 20_000L,
    ) {
        val deadline = System.currentTimeMillis() + timeoutMs

        while (System.currentTimeMillis() < deadline) {
            val node = findObject(By.text(value))
            if (node == null) {
                swipe(displayWidth / 2, displayHeight / 4, displayWidth / 2, displayHeight * 3 / 4, 20)

                // a tap while the list still flings only stops it, so the next tap must wait for it to settle
                waitForIdle()
                continue
            }

            try {
                clickableAncestor(node).click()
                return
            } catch (_: StaleObjectException) {
                Thread.sleep(100)
            }
        }

        error("Timed out clicking text \"$value\"")
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
                "restoreAll=${manager.restoreAllState}, walletLists=${ScriptedCloudStorageAccess.walletListCount()}"

        assertTrue("$message; $diagnostics", condition())
    }

    private companion object {
        const val WALLET_TWO = "Fixture Wallet Two"
        const val WALLET_THREE = "Fixture Wallet Three"
        const val CHECK_AGAIN = "Check Again"
        const val CHECK_AGAIN_ATTEMPTS = 3
        const val RESTORE_ACTION = "Restore to this device"
        const val DELETE_ACTION = "Delete from Cloud Backup"
        const val DELETE_FOREVER = "Delete Forever"
        const val CANCEL = "Cancel"
        val FIXTURE_CLOUD_ONLY_RECORD_IDS =
            listOf(
                ScriptedCloudBackupFixture.WALLET_TWO_RECORD_ID,
                ScriptedCloudBackupFixture.WALLET_THREE_RECORD_ID,
            )
    }
}
