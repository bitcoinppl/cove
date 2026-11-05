package org.bitcoinppl.cove.flows.cloudbackup

import android.graphics.Rect
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.filters.LargeTest
import androidx.test.platform.app.InstrumentationRegistry
import androidx.test.uiautomator.By
import androidx.test.uiautomator.BySelector
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
import org.bitcoinppl.cove_core.CloudOnlyOperation
import org.bitcoinppl.cove_core.CloudOnlyState
import org.bitcoinppl.cove_core.Route
import org.bitcoinppl.cove_core.SettingsRoute
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

/**
 * Deletes one cloud-only wallet backup from Cloud Backup detail through its action sheet and
 * confirmation dialog, against scripted cloud storage that records every delete
 */
@ManualFullLaunchTest
@LargeTest
@RunWith(AndroidJUnit4::class)
class CloudBackupDeleteFromCloudFullLaunchTest {
    private lateinit var device: UiDevice

    @get:Rule
    val fullLaunch = FullLaunchTestRule()

    @Before
    fun prepareDevice() {
        ScriptedPasskeyProvider.reset()
        device = fullLaunchDevice()
    }

    @Test
    fun cancelKeepsEveryBackupAndDeleteForeverRemovesOnlyTheSelectedOne() {
        restoreFixtureAndOpenCloudBackupDetail()
        val localWalletIds = onMainValue(::localWalletIds)
        assertEquals("restore should leave exactly one wallet on this device", 1, localWalletIds.size)

        // the second row, so a delete that took the first row's id would remove the wrong one
        openDeleteConfirmation(SELECTED_WALLET_NAME)
        device.clickText("Cancel")
        assertTrue(
            "Cancel should close the confirmation",
            device.wait(Until.gone(By.text("Delete $SELECTED_WALLET_NAME?")), 10_000L),
        )
        assertNothingDeletedFor(CANCEL_SETTLE_MS)
        assertEquals(
            "Cancel should keep both cloud-only rows",
            listOf(KEPT_RECORD_ID, SELECTED_RECORD_ID),
            onMainValue(::cloudOnlyRecordIds),
        )
        device.waitForText(SELECTED_WALLET_NAME)
        device.waitForText(KEPT_WALLET_NAME)

        openDeleteConfirmation(SELECTED_WALLET_NAME)
        device.clickText("Delete Forever")

        waitUntil("Delete Forever should reach the cloud provider") {
            ScriptedCloudStorageAccess.deletedWalletRecordIds().isNotEmpty() ||
                ScriptedCloudStorageAccess.deleteLocationErrors().isNotEmpty()
        }
        assertEquals(
            "the delete should name exactly the selected record's files",
            emptyList<String>(),
            ScriptedCloudStorageAccess.deleteLocationErrors(),
        )
        assertEquals(
            "only the selected backup should be deleted from the cloud",
            listOf(SELECTED_RECORD_ID),
            ScriptedCloudStorageAccess.deletedWalletRecordIds(),
        )

        // rust drops the row before its detail refresh lands, so wait for the refresh marker row,
        // which only a listing after the delete names, to prove the refreshed inventory was applied
        waitUntil("the refreshed inventory after the delete should be applied") {
            val manager = CloudBackupManager.getInstance()

            manager.isDetailInventoryComplete &&
                manager.cloudOnlyOperation !is CloudOnlyOperation.Operating &&
                REFRESH_MARKER_RECORD_ID in cloudOnlyRecordIds()
        }
        assertEquals(
            "the refreshed inventory should keep only the other cloud-only row and the marker",
            setOf(KEPT_RECORD_ID, REFRESH_MARKER_RECORD_ID),
            onMainValue(::cloudOnlyRecordIds).toSet(),
        )
        device.scrollUntilText(REFRESH_MARKER_WALLET_NAME)
        device.scrollUntilText(KEPT_WALLET_NAME)

        // the manager state check above is the proof; this on-screen check is supplementary
        assertTrue(
            "the deleted wallet should leave Not on This Device",
            device.wait(Until.gone(By.text(SELECTED_WALLET_NAME)), 10_000L),
        )
        assertEquals(
            "only the selected backup should be deleted from the cloud",
            listOf(SELECTED_RECORD_ID),
            ScriptedCloudStorageAccess.deletedWalletRecordIds(),
        )
        assertEquals("the local wallet should stay on this device", localWalletIds, onMainValue(::localWalletIds))
    }

    private fun openDeleteConfirmation(walletName: String) {
        device.clickText(walletName)

        // the sheet slides up, so its action is tapped only once it holds still
        val description = "Restore this wallet to the device or delete it from Cloud Backup."
        device.waitForText(description)
        val deleteAction = device.waitForSettled(By.text("Delete from Cloud Backup"), "Delete from Cloud Backup")
        assertTrue(
            "the action sheet should be titled with the selected wallet",
            sheetContaining(deleteAction, description).hasObject(By.text(walletName)),
        )
        device.clickClickableAncestor(deleteAction)

        device.waitForText("Delete $walletName?")
        device.waitForText("This wallet backup will be permanently removed from Cloud Backup")
        device.waitForText("Delete Forever")
    }

    // the innermost ancestor of the sheet's delete action that also holds its description is the
    // sheet itself, so a search inside it can't match the wallet row underneath
    private fun sheetContaining(
        action: UiObject2,
        description: String,
    ): UiObject2 {
        var node: UiObject2? = action

        while (node != null && !node.hasObject(By.text(description))) {
            node = node.parent
        }

        return requireNotNull(node) { "the delete action is not inside the action sheet" }
    }

    // gives a delete that Cancel would have dispatched time to reach the provider before asserting none did
    private fun assertNothingDeletedFor(durationMs: Long) {
        val deadline = System.currentTimeMillis() + durationMs

        while (System.currentTimeMillis() < deadline) {
            assertEquals(
                "Cancel must not delete any backup",
                emptyList<String>(),
                ScriptedCloudStorageAccess.deletedWalletRecordIds(),
            )
            Thread.sleep(100)
        }

        assertTrue(
            "Cancel must not start a cloud-only operation",
            onMainValue { CloudBackupManager.getInstance().cloudOnlyOperation } is CloudOnlyOperation.Idle,
        )
    }

    private fun restoreFixtureAndOpenCloudBackupDetail() {
        ScriptedCloudStorageAccess.configureProductionFixtureRestore()
        launchFullApp()

        device.waitForText("Google Drive Backup Found")
        device.clickText("Restore with Passkey")
        device.waitForText("You're all set")
        device.waitForText("Your wallets have been restored.")
        ScriptedCloudStorageAccess.exposeAllProductionFixtureWallets()
        device.clickText("Done")
        FullLaunchOnboardingRobot(device).acceptTermsAfterImport()
        waitForAppOnRestoredWallet()

        onMain { AppManager.getInstance().resetRoute(Route.Settings(SettingsRoute.Main)) }
        device.waitForText("Security")
        onMain { AppManager.getInstance().resetRoute(Route.Settings(SettingsRoute.CloudBackup)) }
        device.waitForText("Not on This Device")

        waitUntil("expected the two other fixture wallets as cloud-only rows once the inventory completes") {
            val manager = CloudBackupManager.getInstance()

            manager.isDetailInventoryComplete &&
                manager.isDetailInventoryReady &&
                cloudOnlyRecordIds() == listOf(KEPT_RECORD_ID, SELECTED_RECORD_ID)
        }
        device.scrollUntilText(SELECTED_WALLET_NAME)
        device.waitForText(KEPT_WALLET_NAME)
    }

    // finishing onboarding resets the app's route a moment later, which would replace a Settings
    // route opened before it
    private fun waitForAppOnRestoredWallet() {
        waitUntil("onboarding should finish on the restored wallet") {
            val app = AppManager.getInstance()
            val walletId = app.wallets.singleOrNull()?.id

            walletId != null &&
                app.router.default == Route.SelectedWallet(walletId) &&
                app.router.routes.isEmpty()
        }
    }

    // reads manager state, so call it on the main thread
    private fun cloudOnlyRecordIds(): List<String> =
        ((CloudBackupManager.getInstance().cloudOnly as? CloudOnlyState.Loaded)?.wallets)
            .orEmpty()
            .map { wallet -> wallet.recordId }

    // reads manager state, so call it on the main thread
    private fun localWalletIds(): Set<String> =
        AppManager.getInstance().wallets.mapTo(mutableSetOf()) { wallet -> wallet.id }

    private fun onMain(block: () -> Unit) {
        InstrumentationRegistry.getInstrumentation().runOnMainSync(block)
    }

    private fun <T> onMainValue(block: () -> T): T {
        var value: Result<T>? = null
        onMain { value = runCatching(block) }

        return requireNotNull(value).getOrThrow()
    }

    private fun waitUntil(
        message: String,
        timeoutMs: Long = 30_000L,
        condition: () -> Boolean,
    ) {
        val deadline = System.currentTimeMillis() + timeoutMs

        while (System.currentTimeMillis() < deadline) {
            if (onMainValue(condition)) return

            Thread.sleep(100)
        }

        val diagnostics =
            onMainValue {
                val manager = CloudBackupManager.getInstance()

                "state=${manager.state}, cloudOnly=${cloudOnlyRecordIds()}, " +
                    "operation=${manager.cloudOnlyOperation}, " +
                    "localWallets=${AppManager.getInstance().wallets.map { it.name }}, " +
                    "walletLists=${ScriptedCloudStorageAccess.walletListCount()}, " +
                    "deleted=${ScriptedCloudStorageAccess.deletedWalletRecordIds()}"
            }
        error("$message; $diagnostics")
    }

    private fun UiDevice.waitForText(
        value: String,
        timeoutMs: Long = 20_000L,
    ): UiObject2 =
        wait(Until.findObject(By.text(value)), timeoutMs)
            ?: error("Timed out waiting for text \"$value\", on screen: ${visibleTexts()}")

    private fun UiDevice.visibleTexts(): List<String> = findObjects(By.textStartsWith("")).mapNotNull { it.text }

    private fun UiDevice.clickText(value: String) {
        waitForText(value)
        clickClickableAncestor(waitForSettled(By.text(value), "text \"$value\""))
    }

    private fun UiDevice.scrollUntilText(
        value: String,
        timeoutMs: Long = 20_000L,
    ): UiObject2 {
        val deadline = System.currentTimeMillis() + timeoutMs

        while (System.currentTimeMillis() < deadline) {
            if (hasObject(By.text(value))) return waitForSettled(By.text(value), "text \"$value\"")

            swipe(displayWidth / 2, displayHeight * 3 / 4, displayWidth / 2, displayHeight / 4, 20)
            Thread.sleep(250)
        }

        error("Timed out scrolling to text \"$value\", on screen: ${visibleTexts()}")
    }

    /**
     * Waits until [selector] matches a node whose bounds held still between two looks, so a tap on
     * it lands where it was found rather than where a sliding sheet or dialog has moved it
     */
    private fun UiDevice.waitForSettled(
        selector: BySelector,
        description: String,
        timeoutMs: Long = 20_000L,
    ): UiObject2 {
        val deadline = System.currentTimeMillis() + timeoutMs
        var previous: Rect? = null

        while (System.currentTimeMillis() < deadline) {
            val node = findObject(selector)
            val bounds =
                try {
                    node?.visibleBounds
                } catch (_: StaleObjectException) {
                    null
                }

            if (node != null && bounds != null && bounds == previous) return node

            previous = bounds
            Thread.sleep(SETTLE_POLL_MS)
        }

        error("$description never held still, last bounds $previous\n${visibleTexts()}")
    }

    private fun UiDevice.clickClickableAncestor(node: UiObject2) {
        var clickable: UiObject2? = node

        while (clickable != null && !clickable.isClickable) {
            clickable = clickable.parent
        }

        requireNotNull(clickable) { "expected a clickable ancestor for ${node.text}" }.click()
    }

    private companion object {
        const val SELECTED_RECORD_ID = ScriptedCloudBackupFixture.WALLET_THREE_RECORD_ID
        const val SELECTED_WALLET_NAME = "Fixture Wallet Three"
        const val KEPT_RECORD_ID = ScriptedCloudBackupFixture.WALLET_TWO_RECORD_ID
        const val KEPT_WALLET_NAME = "Fixture Wallet Two"

        // test-only signal: the scripted provider lists this spare fixture wallet only after a
        // delete, so its row shows only once a post-delete detail refresh was applied
        const val REFRESH_MARKER_RECORD_ID = ScriptedCloudBackupFixture.XPRV_WALLET_RECORD_ID
        const val REFRESH_MARKER_WALLET_NAME = ScriptedCloudBackupFixture.XPRV_WALLET_NAME
        const val CANCEL_SETTLE_MS = 3_000L
        const val SETTLE_POLL_MS = 200L
    }
}
