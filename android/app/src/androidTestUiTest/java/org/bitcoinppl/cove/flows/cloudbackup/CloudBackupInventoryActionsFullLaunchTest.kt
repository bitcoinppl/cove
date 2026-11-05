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
import org.bitcoinppl.cove.test.fullLaunchDevice
import org.bitcoinppl.cove.test.launchFullApp
import org.bitcoinppl.cove.testconfig.ScriptedCloudBackupFixture
import org.bitcoinppl.cove.testconfig.ScriptedCloudStorageAccess
import org.bitcoinppl.cove.testconfig.ScriptedPasskeyProvider
import org.bitcoinppl.cove_core.CloudBackupManagerAction
import org.bitcoinppl.cove_core.CloudBackupRestoreAllState
import org.bitcoinppl.cove_core.CloudOnlyState
import org.bitcoinppl.cove_core.Route
import org.bitcoinppl.cove_core.SettingsRoute
import org.bitcoinppl.cove_core.device.CloudStorageException
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

/**
 * Holds the provider's wallet listing while Cloud Backup re-checks a known inventory, and checks
 * that the cloud-only rows stay on screen but cannot open their Restore and Delete actions until
 * the provider confirms the inventory
 */
@ManualFullLaunchTest
@LargeTest
@RunWith(AndroidJUnit4::class)
class CloudBackupInventoryActionsFullLaunchTest {
    private lateinit var device: UiDevice

    @get:Rule
    val fullLaunch = FullLaunchTestRule()

    @Before
    fun prepareDevice() {
        ScriptedPasskeyProvider.reset()
        device = fullLaunchDevice()
    }

    @After
    fun releaseHeldListing() {
        // a failed assertion must not leave the app's listing suspended for the next test
        ScriptedCloudStorageAccess.releaseBlockedWalletList()
    }

    @Test
    fun cloudOnlyRowActionsStayDisabledUntilTheInventoryCheckCompletes() {
        restoreFixtureAndOpenCloudBackupDetail()
        device.scrollUntilText(WALLET_TWO)
        waitUntil("a provider-confirmed row should be enabled before the re-check") {
            device.findObject(By.text(WALLET_TWO))?.let { device.clickableAncestor(it).isEnabled } == true
        }

        // a refresh of a confirmed inventory keeps it complete, so the check starts from a failed
        // refresh, as it does when a device comes back online
        ScriptedCloudStorageAccess.blockNextWalletList()
        dispatchOnMain(CloudBackupManagerAction.RefreshDetail)
        assertTrue(
            "expected the refresh to list wallets from the provider",
            ScriptedCloudStorageAccess.awaitBlockedWalletList(timeoutMs = 20_000L),
        )
        ScriptedCloudStorageAccess.failBlockedWalletList(
            CloudStorageException.Offline("scripted provider is offline"),
        )
        // held before the failure lands, so an automatic retry also waits here
        ScriptedCloudStorageAccess.blockNextWalletList()
        var retriedAutomatically = false
        waitUntil("the failed refresh should keep both rows and disable Restore All") {
            val manager = CloudBackupManager.getInstance()
            retriedAutomatically = ScriptedCloudStorageAccess.awaitBlockedWalletList(timeoutMs = 0L)

            retriedAutomatically ||
                manager.detailError != null &&
                cloudOnlyRecordIds() == FIXTURE_CLOUD_ONLY_RECORD_IDS &&
                manager.restoreAllState is CloudBackupRestoreAllState.StartDisabled
        }

        if (!retriedAutomatically) {
            device.clickClickableAncestorForText(CHECK_AGAIN)
        }
        assertTrue(
            "Check Again should list wallets from the provider again",
            ScriptedCloudStorageAccess.awaitBlockedWalletList(timeoutMs = 20_000L),
        )

        try {
            waitUntil("the held listing should leave the known rows checking") {
                val manager = CloudBackupManager.getInstance()

                manager.isDetailInventoryChecking &&
                    !manager.isDetailInventoryReady &&
                    cloudOnlyRecordIds() == FIXTURE_CLOUD_ONLY_RECORD_IDS
            }

            for (name in listOf(WALLET_TWO, WALLET_THREE)) {
                waitUntil("$name should stay listed but disabled while checking") {
                    device.findObject(By.text(name))?.let { device.clickableAncestor(it).isEnabled } == false
                }
            }

            device.clickableAncestor(device.scrollUntilText(WALLET_TWO)).click()
            assertFalse(
                "a checking row must not open its Restore and Delete actions",
                device.wait(Until.hasObject(By.text(RESTORE_ACTION)), 2_000L),
            )
            assertFalse(
                "a checking row must not open its Restore and Delete actions",
                device.hasObject(By.text(DELETE_ACTION)),
            )
        } finally {
            ScriptedCloudStorageAccess.releaseBlockedWalletList()
        }

        waitUntil("the released listing should complete the inventory with both rows") {
            CloudBackupManager.getInstance().isDetailInventoryComplete &&
                cloudOnlyRecordIds() == FIXTURE_CLOUD_ONLY_RECORD_IDS
        }
        waitUntil("$WALLET_TWO should be enabled once the inventory is complete") {
            device.findObject(By.text(WALLET_TWO))?.let { device.clickableAncestor(it).isEnabled } == true
        }

        device.clickableAncestor(device.scrollUntilText(WALLET_TWO)).click()
        for (action in listOf(RESTORE_ACTION, DELETE_ACTION)) {
            assertTrue(
                "$action should be enabled once the inventory is complete",
                device.clickableAncestor(device.waitForText(action)).isEnabled,
            )
        }
    }

    private fun restoreFixtureAndOpenCloudBackupDetail() {
        ScriptedCloudStorageAccess.configureProductionFixtureRestore()
        launchFullApp()

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

        InstrumentationRegistry.getInstrumentation().runOnMainSync {
            AppManager.getInstance().resetRoute(Route.Settings(SettingsRoute.CloudBackup))
        }
        device.waitForText("Cloud Backup")
        waitUntil("expected two fixture wallets in a provider-confirmed inventory") {
            CloudBackupManager.getInstance().isDetailInventoryComplete &&
                cloudOnlyRecordIds() == FIXTURE_CLOUD_ONLY_RECORD_IDS
        }
        assertEquals(
            "the fixture should restore only its first wallet",
            1,
            AppManager.getInstance().wallets.size,
        )
    }

    private fun dispatchOnMain(action: CloudBackupManagerAction) {
        InstrumentationRegistry.getInstrumentation().runOnMainSync {
            CloudBackupManager.getInstance().dispatch(action)
        }
    }

    private fun cloudOnlyRecordIds(): List<String> =
        ((CloudBackupManager.getInstance().cloudOnly as? CloudOnlyState.Loaded)?.wallets)
            .orEmpty()
            .map { wallet -> wallet.recordId }

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
            Thread.sleep(250)
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

    private fun UiDevice.clickClickableAncestorForText(
        value: String,
        timeoutMs: Long = 20_000L,
    ) {
        val deadline = System.currentTimeMillis() + timeoutMs

        while (System.currentTimeMillis() < deadline) {
            val node = findObject(By.text(value))
            if (node == null) {
                swipe(displayWidth / 2, displayHeight / 4, displayWidth / 2, displayHeight * 3 / 4, 20)
                Thread.sleep(250)
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
            "detail=${manager.state.lifecycle}, ready=${manager.isDetailInventoryReady}, " +
                "complete=${manager.isDetailInventoryComplete}, cloudOnly=${cloudOnlyRecordIds()}, " +
                "restoreAll=${manager.restoreAllState}, walletLists=${ScriptedCloudStorageAccess.walletListCount()}"
        assertTrue("$message; $diagnostics", condition())
    }

    private companion object {
        const val WALLET_TWO = "Fixture Wallet Two"
        const val WALLET_THREE = "Fixture Wallet Three"
        const val CHECK_AGAIN = "Check Again"
        const val RESTORE_ACTION = "Restore to this device"
        const val DELETE_ACTION = "Delete from Cloud Backup"
        val FIXTURE_CLOUD_ONLY_RECORD_IDS =
            listOf(
                ScriptedCloudBackupFixture.WALLET_TWO_RECORD_ID,
                ScriptedCloudBackupFixture.WALLET_THREE_RECORD_ID,
            )
    }
}
