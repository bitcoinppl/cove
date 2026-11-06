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
import org.bitcoinppl.cove.test.FullLaunchStartupRobot
import org.bitcoinppl.cove.test.FullLaunchTestRule
import org.bitcoinppl.cove.test.ManualFullLaunchTest
import org.bitcoinppl.cove.test.TestWalletWords
import org.bitcoinppl.cove.test.deleteTestWallet
import org.bitcoinppl.cove.test.fullLaunchDevice
import org.bitcoinppl.cove.test.importTestWalletOrNull
import org.bitcoinppl.cove.test.launchFullApp
import org.bitcoinppl.cove.testconfig.ScriptedCloudStorageAccess
import org.bitcoinppl.cove.testconfig.ScriptedCloudStorageAccess.UploadKind
import org.bitcoinppl.cove.testconfig.ScriptedPasskeyProvider
import org.bitcoinppl.cove_core.BootstrapStep
import org.bitcoinppl.cove_core.CloudBackupLifecycle
import org.bitcoinppl.cove_core.Route
import org.bitcoinppl.cove_core.SettingsRoute
import org.bitcoinppl.cove_core.bootstrapProgress
import org.bitcoinppl.cove_core.types.WalletId
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotEquals
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

/**
 * Disables Cloud Backup through the detail's production Disable action while the provider holds
 * a wallet upload mid-write
 *
 * The fresh-enable provider stores a held upload only once it is released, so a disable that
 * deleted the backup before the write drained would leave that wallet behind in the cloud
 */
@ManualFullLaunchTest
@LargeTest
@RunWith(AndroidJUnit4::class)
class CloudBackupDisableFullLaunchTest {
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
        // a failed assertion must not leave the upload or provider reads held, or test wallets behind
        ScriptedCloudStorageAccess.releaseBlockedWalletUpload()
        ScriptedCloudStorageAccess.releaseVisibility()
        val launchedWith = walletIdsAtLaunch ?: return

        AppManager
            .getInstance()
            .wallets
            .map { it.id }
            .filterNot(launchedWith::contains)
            .forEach(::deleteTestWallet)
    }

    @Test
    fun disableWaitsForAHeldUploadThenDeletesTheWholeBackup() {
        ScriptedCloudStorageAccess.configureFreshEnableWithDelayedVisibility()
        launchApp()
        enableFreshBackupAndFinishOnboarding()
        val namespace = ScriptedCloudStorageAccess.freshEnableNamespaces().single()

        // adding a wallet uploads it on its own, and the provider parks that upload before storing it
        ScriptedCloudStorageAccess.blockNextWalletUpload()
        val uploadsBeforeHeld = ScriptedCloudStorageAccess.uploadCount(UploadKind.WALLET)
        requireNotNull(importTestWalletOrNull(HELD_WALLET)) { "a freshly reset app should not hold the test wallet" }
        assertTrue("the new wallet's upload never reached the provider", ScriptedCloudStorageAccess.awaitBlockedWalletUpload())

        openCloudBackup()
        val manager = CloudBackupManager.getInstance()
        waitUntil("the detail never confirmed its inventory, which Disable needs") { manager.isDetailInventoryComplete }
        disableThroughBothConfirmations()

        waitUntil("the disable never started") { manager.isDisablingCloudBackup }
        assertBackupKeptWhileWriteIsHeld(namespace)

        ScriptedCloudStorageAccess.releaseBlockedWalletUpload()

        waitUntil("the disable should finish once the held write drains", timeoutMs = 60_000L) {
            manager.lifecycle is CloudBackupLifecycle.Disabled
        }
        assertEquals("the disable should delete the backup once", 1, ScriptedCloudStorageAccess.namespaceDeleteCount())
        assertEquals(
            "the disable left a master key in the cloud",
            emptyList<String>(),
            ScriptedCloudStorageAccess.freshEnableNamespaces(),
        )
        assertEquals(
            "a write that finished after the delete left a wallet in the cloud",
            emptyList<String>(),
            ScriptedCloudStorageAccess.freshEnableWalletRecordIds(namespace),
        )
        assertEquals(
            "the disable let another wallet upload through",
            uploadsBeforeHeld + 1,
            ScriptedCloudStorageAccess.uploadCount(UploadKind.WALLET),
        )
    }

    private fun disableThroughBothConfirmations() {
        device.clickText(DISABLE_CLOUD_BACKUP, scroll = true)
        device.waitForText("Disable Cloud Backup?")
        device.clickText("Continue")
        device.waitForText("Delete Cloud Backups?")
        device.clickText("Delete Cloud Backups and Disable")
    }

    /** The disable keeps waiting on the held write for a while, deleting nothing */
    private fun assertBackupKeptWhileWriteIsHeld(namespace: String) {
        val deadline = System.currentTimeMillis() + HELD_WRITE_OBSERVATION_MS

        while (System.currentTimeMillis() < deadline) {
            assertEquals(
                "the disable deleted the backup before the write drained",
                0,
                ScriptedCloudStorageAccess.namespaceDeleteCount(),
            )
            assertEquals(
                "the backup disappeared while the write was held",
                listOf(namespace),
                ScriptedCloudStorageAccess.freshEnableNamespaces(),
            )
            assertNotEquals(
                "the disable finished before the write drained",
                CloudBackupLifecycle.Disabled,
                CloudBackupManager.getInstance().lifecycle,
            )

            Thread.sleep(200)
        }
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
        waitUntil("expected background upload confirmation to settle", timeoutMs = 60_000L) {
            !CloudBackupManager.getInstance().hasPendingUploadVerification
        }

        device.clickText("Continue")
        onboarding.continueFromBackupWallet().acceptTermsAfterImport()
        assertTrue(
            "accepting terms should complete onboarding before opening Settings",
            device.wait(Until.gone(By.text("Terms & Conditions")), 10_000L),
        )
    }

    private fun launchApp() {
        // the reset only runs in a fresh activity, which the test orchestrator's process per test gives
        launchFullApp()
        awaitBootstrap()
        walletIdsAtLaunch = AppManager.getInstance().wallets.map { it.id }.toSet()
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

    private fun UiDevice.waitForText(
        value: String,
        timeoutMs: Long = 20_000L,
    ): UiObject2 =
        wait(Until.findObject(By.text(value)), timeoutMs)
            ?: error("Timed out waiting for text \"$value\"")

    private fun UiDevice.clickText(
        value: String,
        scroll: Boolean = false,
        timeoutMs: Long = 20_000L,
    ) {
        val deadline = System.currentTimeMillis() + timeoutMs

        while (System.currentTimeMillis() < deadline) {
            val node = wait(Until.findObject(By.text(value)), if (scroll) 1_000L else timeoutMs)
            if (node != null) {
                clickClickableAncestor(node)
                return
            }

            if (scroll) {
                swipe(displayWidth / 2, displayHeight * 3 / 4, displayWidth / 2, displayHeight / 4, 20)

                // a tap while the list still flings only stops it, so the next tap must wait for it to settle
                waitForIdle()
            }
        }

        error("Timed out clicking text \"$value\"")
    }

    private fun clickClickableAncestor(node: UiObject2) {
        var clickable: UiObject2? = node

        while (clickable != null && !clickable.isClickable) {
            clickable = clickable.parent
        }

        requireNotNull(clickable) { "expected a clickable ancestor for ${node.text}" }.click()
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
            "lifecycle=${manager.lifecycle}, row=${manager.settingsRowStatus}, " +
                "namespaces=${ScriptedCloudStorageAccess.freshEnableNamespaces()}, " +
                "namespaceDeletes=${ScriptedCloudStorageAccess.namespaceDeleteCount()}, " +
                "uploads=${ScriptedCloudStorageAccess.uploads()}"

        assertTrue("$message; $diagnostics", condition())
    }

    private companion object {
        const val DISABLE_CLOUD_BACKUP = "Disable Cloud Backup"
        const val HELD_WRITE_OBSERVATION_MS = 3_000L

        // a published BIP39 test vector, never used for funds
        val HELD_WALLET =
            TestWalletWords(
                words = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about"
                    .split(" "),
                masterFingerprint = "73c5da0a",
            )
    }
}
