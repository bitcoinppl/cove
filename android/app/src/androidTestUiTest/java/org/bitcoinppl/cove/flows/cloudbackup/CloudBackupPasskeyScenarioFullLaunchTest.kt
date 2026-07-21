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
import org.bitcoinppl.cove.test.fullLaunchDevice
import org.bitcoinppl.cove.test.launchFullApp
import org.bitcoinppl.cove.testconfig.ScriptedCloudStorageAccess
import org.bitcoinppl.cove.testconfig.ScriptedPasskeyProvider
import org.bitcoinppl.cove.testconfig.ScriptedPasskeyProvider.Invocation
import org.bitcoinppl.cove.testconfig.ScriptedPasskeyProvider.Result
import org.bitcoinppl.cove_core.Route
import org.bitcoinppl.cove_core.RustConnectivityManager
import org.bitcoinppl.cove_core.SettingsRoute
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
class CloudBackupPasskeyScenarioFullLaunchTest {
    private companion object {
        const val NO_RETRY_SETTLE_DELAY_MS = 1_000L
        const val PROVIDER_WRITE_CHECK_TIMEOUT_MS = 250L
        const val EXPECTED_DISCOVERY_ATTEMPTS = 2
        const val PASSKEY_RETRY_TIMEOUT_MS = 30_000L
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
    fun postPresentationFailureDoesNotRetry() {
        ScriptedCloudStorageAccess.configureFreshEnableWithDelayedVisibility()
        ScriptedPasskeyProvider.configureResults(
            Invocation.CREATE,
            Result.POST_PRESENTATION_FAILURE,
            Result.SUCCESS,
        )
        launchFullApp()

        FullLaunchStartupRobot(device).assertBootstrappedIntoOnboarding()
        FullLaunchOnboardingRobot(device)
            .tapGetStarted()
            .chooseNewUser()
            .openCloudBackupFromBackupWallet()
            .assertCloudBackupDetails()
            .enableCloudBackupFromDetailsOnce()

        assertTrue("expected passkey creation to be attempted", ScriptedPasskeyProvider.awaitCreation())
        Thread.sleep(NO_RETRY_SETTLE_DELAY_MS)

        assertEquals(
            "post-presentation failure must not start a second passkey request",
            1,
            ScriptedPasskeyProvider.callCount(Invocation.CREATE),
        )
        assertFalse(
            "a rejected passkey must stop before provider writes",
            ScriptedCloudStorageAccess.awaitMasterWriteAccepted(
                PROVIDER_WRITE_CHECK_TIMEOUT_MS,
            ),
        )
        FullLaunchOnboardingRobot(device).assertCloudBackupDetails()
    }

    @Test
    fun prePresentationDiscoveryFailureRetriesOnceThenSucceeds() {
        ScriptedCloudStorageAccess.configureProductionFixtureRestore()
        ScriptedPasskeyProvider.configureResults(
            Invocation.DISCOVER,
            Result.PRE_PRESENTATION_FAILURE,
            Result.SUCCESS,
        )
        launchFullApp()

        device.waitForText("Google Drive Backup Found")
        device.clickClickableAncestor(device.waitForText("Restore with Passkey"))

        assertTrue(
            "expected one bounded discovery retry to reach the scripted success; " +
                "discover=${ScriptedPasskeyProvider.callCount(Invocation.DISCOVER)}, " +
                "authenticate=${ScriptedPasskeyProvider.callCount(Invocation.AUTHENTICATE)}, " +
                "create=${ScriptedPasskeyProvider.callCount(Invocation.CREATE)}, " +
                "connectivity=${rustConnectivityStatus()}, " +
                "masterDownloads=${ScriptedCloudStorageAccess.masterDownloadCount()}, " +
                "walletLists=${ScriptedCloudStorageAccess.walletListCount()}, " +
                "walletDownloads=${ScriptedCloudStorageAccess.walletDownloadCount()}",
            ScriptedPasskeyProvider.awaitCallCount(
                Invocation.DISCOVER,
                expected = EXPECTED_DISCOVERY_ATTEMPTS,
                timeoutMs = PASSKEY_RETRY_TIMEOUT_MS,
            ),
        )
        assertEquals(
            EXPECTED_DISCOVERY_ATTEMPTS,
            ScriptedPasskeyProvider.callCount(Invocation.DISCOVER),
        )
        assertEquals(
            "discovery recovery must not fall through to registration",
            0,
            ScriptedPasskeyProvider.callCount(Invocation.CREATE),
        )
        assertEquals(
            "the discovered credential and PRF result should complete the passkey operation",
            0,
            ScriptedPasskeyProvider.callCount(Invocation.AUTHENTICATE),
        )
        device.waitForText("You're all set")
        device.waitForText("Your wallets have been restored.")
    }

    @Test
    fun providerSignalWhileRefreshIsInFlightRunsOneTrailingRefresh() {
        ScriptedCloudStorageAccess.configureFreshEnableWithDelayedVisibility()
        launchFullApp()

        enableFreshBackupAndFinishOnboarding()
        device.openSettings()

        val walletListsBeforeRefresh = ScriptedCloudStorageAccess.walletListCount()
        ScriptedCloudStorageAccess.blockNextWalletList()
        device.openCloudBackup()

        assertTrue(
            "expected detail entry to start a provider inventory refresh",
            ScriptedCloudStorageAccess.awaitBlockedWalletList(),
        )

        CloudBackupManager.getInstance().refreshCloudState()
        ScriptedCloudStorageAccess.releaseBlockedWalletList()

        assertTrue(
            "provider signal during the blocked refresh must run one trailing refresh; " +
                "before=$walletListsBeforeRefresh, after=${ScriptedCloudStorageAccess.walletListCount()}",
            ScriptedCloudStorageAccess.awaitWalletListCount(walletListsBeforeRefresh + 2),
        )
        waitUntil("detail refresh should settle after the trailing provider-signal refresh") {
            !CloudBackupManager.getInstance().isDetailInventoryChecking
        }
        assertEquals(
            "provider signal should coalesce to exactly one trailing refresh",
            walletListsBeforeRefresh + 2,
            ScriptedCloudStorageAccess.walletListCount(),
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
            "expected the configured backup to become authoritatively visible",
            ScriptedCloudStorageAccess.awaitVisibleConfirmationRead(),
        )
        waitUntil("expected background upload confirmation to settle") {
            !CloudBackupManager.getInstance().hasPendingUploadVerification
        }

        device.clickClickableAncestor(device.waitForText("Continue"))
        onboarding.continueFromBackupWallet().acceptTermsAfterImport()
        assertTrue(
            "accepting terms should complete onboarding before opening Cloud Backup settings",
            device.wait(Until.gone(By.text("Terms & Conditions")), 10_000L),
        )
    }

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
        timeoutMs: Long = 20_000,
    ) = wait(Until.findObject(By.text(value)), timeoutMs)
        ?: error("Timed out waiting for text \"$value\"")

    private fun UiDevice.clickClickableAncestor(node: UiObject2) {
        var clickable: UiObject2? = node

        while (clickable != null && !clickable.isClickable) {
            clickable = clickable.parent
        }

        requireNotNull(clickable) { "expected a clickable ancestor for ${node.text}" }.click()
    }

    private fun rustConnectivityStatus() =
        RustConnectivityManager().use { manager ->
            manager.state().status
        }

    private fun waitUntil(
        message: String,
        timeoutMs: Long = 20_000L,
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
