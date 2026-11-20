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
import org.bitcoinppl.cove.test.FullLaunchStartupRobot
import org.bitcoinppl.cove.test.FullLaunchTestRule
import org.bitcoinppl.cove.test.ManualFullLaunchTest
import org.bitcoinppl.cove.test.fullLaunchDevice
import org.bitcoinppl.cove.testconfig.ScriptedCloudStorageAccess
import org.bitcoinppl.cove.testconfig.ScriptedPasskeyProvider
import org.bitcoinppl.cove_core.CloudBackupWalletRecovery
import org.bitcoinppl.cove_core.Route
import org.bitcoinppl.cove_core.SettingsRoute
import org.bitcoinppl.cove_core.WalletSettingsRoute
import org.bitcoinppl.cove_core.types.WalletId
import org.junit.After
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

/**
 * A wallet counts as recoverable only once Cloud Backup confirms its copy, not as soon as Cloud
 * Backup is on, though Cloud Backup being on hides its backup reminder from the start
 *
 * The provider accepts the fresh backup but keeps it unreadable, so its upload stays unconfirmed:
 * the wallet shows no backup reminder, but deleting it warns that its funds are lost. Once the
 * provider shows the backup and it confirms, even while a delete prompt is open, the warning
 * points to the cloud copy
 */
@ManualFullLaunchTest
@LargeTest
@RunWith(AndroidJUnit4::class)
class CloudBackupWalletRecoveryFullLaunchTest {
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
    fun releaseProvider() {
        // a failed assertion must not leave provider reads held
        ScriptedCloudStorageAccess.releaseVisibility()
    }

    @Test
    fun aWalletIsBackedUpOnlyOnceCloudBackupConfirmsIt() {
        ScriptedCloudStorageAccess.configureFreshEnableWithDelayedVisibility()
        robot.launchApp(resetData = true)
        enableCloudBackupAndFinishOnboardingUnconfirmed()

        val wallet = AppManager.getInstance().wallets.single()
        assertFalse("the fresh wallet's words should be unverified", wallet.verified)
        assertTrue(
            "the backup should still wait for its confirmation",
            CloudBackupManager.getInstance().hasPendingUploadVerification,
        )

        openWallet(wallet.id)
        assertNoBackupReminder("Cloud Backup is on, so the reminder must not show even before the upload confirms")
        assertFinalDeleteWarning(wallet.id, message = NOT_BACKED_UP_MESSAGE, button = DELETE_FOREVER)

        // the backup confirms while the first delete prompt is open, and the warning it leads to
        // must say so rather than what was true when the prompt opened
        assertFinalDeleteWarning(wallet.id, message = CLOUD_RECOVERY_MESSAGE, button = DELETE) {
            ScriptedCloudStorageAccess.releaseVisibility()
            CloudBackupManager.getInstance().resumePendingCloudUploadVerification()
            robot.waitUntil("the backup never confirmed", timeoutMs = 60_000L) {
                CloudBackupManager.getInstance().recovery.wallet(wallet.id) == CloudBackupWalletRecovery.CONFIRMED
            }
        }

        openWallet(wallet.id)
        assertNoBackupReminder("a confirmed backup must keep the reminder hidden")
    }

    // the balance proves the wallet screen rendered, so the reminder's absence is not just a
    // screen still loading
    private fun assertNoBackupReminder(message: String) {
        assertTrue(
            "the wallet screen never rendered",
            device.wait(Until.hasObject(By.res("selectedWallet.balance")), 10_000L),
        )
        assertFalse(message, device.hasObject(By.text(BACKUP_REMINDER)))
    }

    // the uploads are accepted but stay unreadable until the test releases them, so onboarding
    // finishes with the backup on and its first upload unconfirmed
    private fun enableCloudBackupAndFinishOnboardingUnconfirmed() {
        FullLaunchStartupRobot(device).assertBootstrappedIntoOnboarding()
        val onboarding =
            FullLaunchOnboardingRobot(device)
                .tapGetStarted()
                .chooseNewUser()
                .openCloudBackupFromBackupWallet()
                .assertCloudBackupDetails()
                .enableCloudBackupFromDetails()

        assertTrue(
            "the provider should accept the fresh master and wallet backups",
            ScriptedCloudStorageAccess.awaitAllProviderWritesAccepted(),
        )
        onboarding.assertCloudBackupSuccess()

        robot.clickText("Continue")
        onboarding.continueFromBackupWallet().acceptTermsAfterImport()
        assertTrue(
            "accepting terms should complete onboarding",
            device.wait(Until.gone(By.text("Terms & Conditions")), 10_000L),
        )

        // finishing onboarding resets the route a moment later, which would replace one opened before it
        robot.waitUntil("onboarding should finish on a wallet") {
            val router = AppManager.getInstance().router
            router.default is Route.SelectedWallet && router.routes.isEmpty()
        }
    }

    private fun openWallet(walletId: WalletId) {
        InstrumentationRegistry.getInstrumentation().runOnMainSync {
            AppManager.getInstance().resetRoute(Route.SelectedWallet(walletId))
        }

        robot.waitUntil("the wallet never opened") {
            AppManager.getInstance().currentRoute == Route.SelectedWallet(walletId)
        }
    }

    // a new hot wallet whose words were never verified reaches the Final Warning, the only dialog
    // that says whether Cloud Backup can bring it back; `whileFirstPromptOpen` runs while the
    // first prompt shows
    private fun assertFinalDeleteWarning(
        walletId: WalletId,
        message: String,
        button: String,
        whileFirstPromptOpen: () -> Unit = {},
    ) {
        InstrumentationRegistry.getInstrumentation().runOnMainSync {
            AppManager.getInstance().resetRoute(Route.Settings(SettingsRoute.Main))
        }

        // a push before Settings shows is replaced once the reset lands
        robot.waitForText("Security")

        // pushed onto Settings, the same stack tapping the wallet's row builds
        InstrumentationRegistry.getInstrumentation().runOnMainSync {
            AppManager.getInstance().pushRoute(
                Route.Settings(SettingsRoute.Wallet(id = walletId, route = WalletSettingsRoute.MAIN)),
            )
        }

        robot.scrollToText(DELETE_WALLET_ITEM)
        robot.clickText(DELETE_WALLET_ITEM)

        for (title in listOf("Are you sure?", "Confirm Deletion")) {
            robot.waitForText(title)
            if (title == "Are you sure?") whileFirstPromptOpen()

            robot.clickText(DELETE)
            assertTrue("$title should close once confirmed", device.wait(Until.gone(By.text(title)), 10_000L))
        }

        robot.waitForText(FINAL_WARNING)
        assertTrue(
            "the final warning should start with \"$message\"",
            device.hasObject(By.textStartsWith(message)),
        )
        assertTrue("the final warning's button should read \"$button\"", device.hasObject(By.text(button)))

        robot.clickText(CANCEL)
        assertTrue("cancelling should keep the wallet", device.wait(Until.gone(By.text(FINAL_WARNING)), 10_000L))
        assertTrue(
            "cancelling should keep the wallet",
            AppManager.getInstance().wallets.any { it.id == walletId },
        )
    }

    private companion object {
        const val BACKUP_REMINDER = "Backup your wallet"
        const val DELETE_WALLET_ITEM = "Delete wallet"
        const val DELETE = "Delete"
        const val DELETE_FOREVER = "Delete Forever"
        const val CANCEL = "Cancel"
        const val FINAL_WARNING = "Final Warning"
        const val NOT_BACKED_UP_MESSAGE = "This wallet is not backed up"
        const val CLOUD_RECOVERY_MESSAGE = "This wallet will be deleted from this device. You can recover it"
    }
}
