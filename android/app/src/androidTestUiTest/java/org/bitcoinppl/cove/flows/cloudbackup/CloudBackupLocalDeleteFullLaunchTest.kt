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
import org.bitcoinppl.cove.test.FullLaunchStartupRobot
import org.bitcoinppl.cove.test.FullLaunchTestRule
import org.bitcoinppl.cove.test.ManualFullLaunchTest
import org.bitcoinppl.cove.test.deleteTestWallet
import org.bitcoinppl.cove.test.fullLaunchDevice
import org.bitcoinppl.cove.test.launchFullApp
import org.bitcoinppl.cove.testconfig.ScriptedCloudStorageAccess
import org.bitcoinppl.cove.testconfig.ScriptedPasskeyProvider
import org.bitcoinppl.cove_core.CloudBackupWalletItem
import org.bitcoinppl.cove_core.CloudBackupWalletStatus
import org.bitcoinppl.cove_core.CloudOnlyState
import org.bitcoinppl.cove_core.Route
import org.bitcoinppl.cove_core.SettingsRoute
import org.bitcoinppl.cove_core.WalletSettingsRoute
import org.bitcoinppl.cove_core.types.WalletId
import org.junit.After
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

/**
 * Deletes a cloud-backed wallet from this device through Wallet Settings and its confirmation
 * dialogs, then checks that Cloud Backup lists the cloud copy as restorable and restores it, both
 * for a wallet restored from the fixture backup and for one backed up by a fresh enable
 */
@ManualFullLaunchTest
@LargeTest
@RunWith(AndroidJUnit4::class)
class CloudBackupLocalDeleteFullLaunchTest {
    private lateinit var device: UiDevice

    @get:Rule
    val fullLaunch = FullLaunchTestRule()

    @Before
    fun prepareDevice() {
        ScriptedPasskeyProvider.reset()
        device = fullLaunchDevice()
    }

    @After
    fun cleanUp() {
        // a failed assertion must not leave provider reads held or test wallets behind
        ScriptedCloudStorageAccess.releaseVisibility()
        ScriptedCloudStorageAccess.releaseMasterUploadReturn()
        AppManager.getInstance().wallets.map { it.id }.forEach(::deleteTestWallet)
    }

    @Test
    fun deletingTheLocalWalletKeepsARestorableCloudOnlyCopy() {
        restoreFixtureWallet()

        deleteThroughWalletSettingsThenRestoreFromCloudBackup()
    }

    @Test
    fun deletingAFreshlyBackedUpWalletKeepsItsProviderCopyRestorable() {
        // this provider keeps what the app writes and drops what it deletes, so a stray cloud
        // delete would leave nothing to list or restore
        ScriptedCloudStorageAccess.configureFreshEnableWithDelayedVisibility()
        launchFullApp()
        enableFreshBackupAndFinishOnboarding()

        deleteThroughWalletSettingsThenRestoreFromCloudBackup()
    }

    private fun deleteThroughWalletSettingsThenRestoreFromCloudBackup() {
        val wallet = AppManager.getInstance().wallets.single()
        var recordId: String? = null

        openCloudBackup()
        waitUntil("the wallet should be backed up and nothing should be cloud-only") {
            val manager = CloudBackupManager.getInstance()
            recordId = manager.detail?.upToDate?.singleOrNull()?.recordId

            manager.isDetailInventoryComplete && recordId != null && cloudOnlyWallets().isEmpty()
        }

        openWalletSettings(wallet.id)
        device.clickClickableAncestorForText(DELETE_WALLET_ITEM)
        confirmEveryDeletionDialog()

        waitUntil("the wallet should leave this device") {
            AppManager.getInstance().wallets.none { it.id == wallet.id }
        }

        openCloudBackup()
        waitUntil("the provider should still list the deleted wallet's backup as restorable") {
            val restorable = cloudOnlyWallets().singleOrNull()

            CloudBackupManager.getInstance().isDetailInventoryComplete &&
                restorable?.recordId == recordId &&
                restorable?.syncStatus == CloudBackupWalletStatus.DELETED_FROM_DEVICE &&
                restorable?.restoreFailure == null
        }

        device.scrollUntilText(NOT_ON_THIS_DEVICE)
        device.clickClickableAncestorForText(wallet.name)
        device.clickClickableAncestorForText(RESTORE_ACTION)

        waitUntil("restoring the cloud-only row should bring the wallet back") {
            cloudOnlyWallets().isEmpty() &&
                AppManager.getInstance().wallets.any { it.id == wallet.id && it.name == wallet.name }
        }

        assertTrue(
            "the restored wallet should leave the Not on This Device section",
            device.wait(Until.gone(By.text(NOT_ON_THIS_DEVICE)), 10_000L),
        )
    }

    private fun enableFreshBackupAndFinishOnboarding() {
        FullLaunchStartupRobot(device).assertBootstrappedIntoOnboarding()
        val onboarding =
            FullLaunchOnboardingRobot(device)
                .tapGetStarted()
                .chooseNewUser()
                .openCloudBackupFromBackupWallet()
                .assertCloudBackupDetails()
                .enableCloudBackupFromDetails()

        assertTrue(
            "expected the provider to accept the fresh master and wallet backups",
            ScriptedCloudStorageAccess.awaitAllProviderWritesAccepted(),
        )

        onboarding.assertCloudBackupSuccess()

        ScriptedCloudStorageAccess.releaseVisibility()
        CloudBackupManager.getInstance().resumePendingCloudUploadVerification()
        assertTrue(
            "expected the new backup to become visible to the app",
            ScriptedCloudStorageAccess.awaitVisibleConfirmationRead(),
        )

        waitUntil("expected background upload confirmation to settle") {
            !CloudBackupManager.getInstance().hasPendingUploadVerification
        }

        device.clickClickableAncestor(device.waitForText("Continue"))
        onboarding.continueFromBackupWallet().acceptTermsAfterImport()
        assertTrue(
            "accepting terms should complete onboarding before opening Settings",
            device.wait(Until.gone(By.text("Terms & Conditions")), 10_000L),
        )
    }

    private fun restoreFixtureWallet() {
        ScriptedCloudStorageAccess.configureProductionFixtureRestore()
        launchFullApp()

        device.waitForText("Google Drive Backup Found")
        device.clickClickableAncestor(device.waitForText("Restore with Passkey"))
        device.waitForText("You're all set")
        device.clickClickableAncestor(device.waitForText("Done"))
        FullLaunchOnboardingRobot(device).acceptTermsAfterImport()
        assertTrue(
            "accepting terms should complete onboarding before opening Settings",
            device.wait(Until.gone(By.text("Terms & Conditions")), 10_000L),
        )
    }

    // taps through however many confirmations the wallet needs, checking each dialog's wording
    private fun confirmEveryDeletionDialog() {
        device.waitForText("Are you sure?")
        device.clickText(DELETE_BUTTON)

        val deadline = System.currentTimeMillis() + 20_000L

        while (System.currentTimeMillis() < deadline) {
            when {
                device.hasObject(By.text("Confirm Deletion")) -> {
                    device.clickText(DELETE_BUTTON)
                    device.wait(Until.gone(By.text("Confirm Deletion")), 10_000L)
                }

                device.hasObject(By.text("Final Warning")) -> {
                    // with Cloud Backup on, the last dialog must promise the cloud copy survives
                    assertTrue(
                        "the final warning should point to the cloud copy",
                        device.hasObject(By.textStartsWith(CLOUD_RECOVERY_MESSAGE)),
                    )

                    device.clickText(DELETE_BUTTON)
                    device.wait(Until.gone(By.text("Final Warning")), 10_000L)
                }

                device.hasObject(By.text("Failed to Delete Wallet")) -> {
                    error("wallet deletion showed Failed to Delete Wallet")
                }

                AppManager.getInstance().wallets.isEmpty() -> return

                else -> Thread.sleep(100)
            }
        }

        error("Timed out confirming wallet deletion")
    }

    private fun cloudOnlyWallets(): List<CloudBackupWalletItem> =
        (CloudBackupManager.getInstance().cloudOnly as? CloudOnlyState.Loaded)?.wallets.orEmpty()

    private fun openCloudBackup() {
        InstrumentationRegistry.getInstrumentation().runOnMainSync {
            AppManager.getInstance().resetRoute(Route.Settings(SettingsRoute.CloudBackup))
        }

        device.waitForText("Cloud Backup")
    }

    private fun openWalletSettings(walletId: WalletId) {
        InstrumentationRegistry.getInstrumentation().runOnMainSync {
            AppManager.getInstance().resetRoute(Route.Settings(SettingsRoute.Main))
        }

        device.waitForText("Security")

        // pushed onto Settings, the same stack tapping the wallet's row builds
        InstrumentationRegistry.getInstrumentation().runOnMainSync {
            AppManager.getInstance().pushRoute(
                Route.Settings(SettingsRoute.Wallet(id = walletId, route = WalletSettingsRoute.MAIN)),
            )
        }

        device.scrollUntilText(DELETE_WALLET_ITEM)
    }

    private fun UiDevice.waitForText(
        value: String,
        timeoutMs: Long = 20_000L,
    ): UiObject2 =
        wait(Until.findObject(By.text(value)), timeoutMs)
            ?: error("Timed out waiting for text \"$value\"")

    private fun UiDevice.clickText(value: String) {
        clickClickableAncestor(waitForText(value))
    }

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

    private fun UiDevice.clickClickableAncestor(node: UiObject2) {
        var clickable: UiObject2? = node

        while (clickable != null && !clickable.isClickable) {
            clickable = clickable.parent
        }

        requireNotNull(clickable) { "expected a clickable ancestor for ${node.text}" }.click()
    }

    private fun UiDevice.clickClickableAncestorForText(
        value: String,
        timeoutMs: Long = 20_000L,
    ) {
        val deadline = System.currentTimeMillis() + timeoutMs

        while (System.currentTimeMillis() < deadline) {
            val node = findObject(By.text(value))
            if (node == null) {
                swipe(displayWidth / 2, displayHeight * 3 / 4, displayWidth / 2, displayHeight / 4, 20)
                Thread.sleep(250)
                continue
            }

            try {
                clickClickableAncestor(node)
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
            "detail=${manager.state.lifecycle}, cloudOnly=${cloudOnlyWallets()}, " +
                "localWallets=${AppManager.getInstance().wallets.map { it.name }}, " +
                "walletLists=${ScriptedCloudStorageAccess.walletListCount()}, " +
                "walletDownloads=${ScriptedCloudStorageAccess.walletDownloadRecordIds()}"
        assertTrue("$message; $diagnostics", condition())
    }

    private companion object {
        const val DELETE_WALLET_ITEM = "Delete wallet"
        const val DELETE_BUTTON = "Delete"
        const val RESTORE_ACTION = "Restore to this device"
        const val NOT_ON_THIS_DEVICE = "Not on This Device"
        const val CLOUD_RECOVERY_MESSAGE = "This wallet will be deleted from this device. You can recover it"
    }
}
