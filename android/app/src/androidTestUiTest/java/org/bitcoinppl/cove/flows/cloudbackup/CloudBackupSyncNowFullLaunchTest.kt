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
import org.bitcoinppl.cove.test.TestWalletWords
import org.bitcoinppl.cove.test.fullLaunchDevice
import org.bitcoinppl.cove.test.importTestWalletOrNull
import org.bitcoinppl.cove.test.launchFullApp
import org.bitcoinppl.cove.testconfig.ScriptedCloudStorageAccess
import org.bitcoinppl.cove.testconfig.ScriptedConnectivityAccess
import org.bitcoinppl.cove.testconfig.ScriptedPasskeyProvider
import org.bitcoinppl.cove.testconfig.ScriptedPasskeyProvider.Invocation
import org.bitcoinppl.cove.testconfig.ScriptedPasskeyProvider.Result
import org.bitcoinppl.cove_core.CloudBackupSyncState
import org.bitcoinppl.cove_core.CloudBackupVerificationState
import org.bitcoinppl.cove_core.CloudBackupWalletStatus
import org.bitcoinppl.cove_core.Route
import org.bitcoinppl.cove_core.SettingsRoute
import org.junit.After
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
class CloudBackupSyncNowFullLaunchTest {
    private companion object {
        const val SYNC_NOW = "Sync Now"
        const val DETAIL_SCAN_SWIPES = 6

        // a published BIP39 test vector, never used for funds
        val PENDING_WALLET =
            TestWalletWords(
                words = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about"
                    .split(" "),
                masterFingerprint = "73c5da0a",
            )
    }

    private lateinit var device: UiDevice

    @get:Rule
    val fullLaunch = FullLaunchTestRule()

    @Before
    fun prepareDevice() {
        ScriptedPasskeyProvider.reset()
        ScriptedConnectivityAccess.reset()
        device = fullLaunchDevice()
    }

    @After
    fun restoreConnectivity() {
        ScriptedConnectivityAccess.reset()
    }

    @Test
    fun syncNowUploadsPendingWalletAndRefreshesDetail() {
        val pendingWalletName = restoreFixtureWithPendingWallet()

        device.clickText(SYNC_NOW)

        waitUntil("Sync Now should upload the pending wallet") {
            ScriptedCloudStorageAccess.acceptedNewWalletUploadCount() == 1
        }
        assertSyncedDetailShows(pendingWalletName)
    }

    @Test
    fun offlineSyncNowUploadsNothingThenUploadsAfterReconnect() {
        val pendingWalletName = restoreFixtureWithPendingWallet()

        ScriptedConnectivityAccess.setConnected(false)
        device.clickText(SYNC_NOW)

        waitUntil("Sync Now offline should fail with the offline error") {
            (CloudBackupManager.getInstance().syncState as? CloudBackupSyncState.Failed)
                ?.v1
                ?.contains("Reconnect to the internet") == true
        }
        device.waitForTextContaining("Reconnect to the internet")
        assertEquals(
            "nothing may be uploaded while offline",
            0,
            ScriptedCloudStorageAccess.acceptedNewWalletUploadCount(),
        )

        ScriptedConnectivityAccess.setConnected(true)
        device.clickText(SYNC_NOW)

        waitUntil("Sync Now after reconnecting should upload the pending wallet") {
            ScriptedCloudStorageAccess.acceptedNewWalletUploadCount() == 1
        }
        assertSyncedDetailShows(pendingWalletName)
    }

    @Test
    fun cancelledVerificationRetriesWithTheSamePasskey() {
        restoreFixtureAndOpenCloudBackupDetail()
        waitUntil("restore should leave the backup verified") {
            CloudBackupManager.getInstance().verificationState is CloudBackupVerificationState.Verified
        }
        val createsBefore = ScriptedPasskeyProvider.callCount(Invocation.CREATE)
        val assertionsBefore = passkeyAssertionCount()
        val authenticationsBefore = ScriptedPasskeyProvider.authenticatedCredentialIds().size
        ScriptedPasskeyProvider.configureResults(Invocation.AUTHENTICATE, Result.USER_CANCELLED, Result.SUCCESS)
        ScriptedPasskeyProvider.configureResults(Invocation.DISCOVER, Result.USER_CANCELLED, Result.SUCCESS)

        device.clickText("Verify Again")

        device.scrollUntilText("Cloud Backup Not Verified")
        waitUntil("cancelling the passkey should leave verification cancelled") {
            CloudBackupManager.getInstance().verificationState == CloudBackupVerificationState.Cancelled
        }
        assertEquals(assertionsBefore + 1, passkeyAssertionCount())

        device.clickText("Verify Now")

        waitUntil("verifying again should verify the backup with the same passkey") {
            CloudBackupManager.getInstance().verificationState is CloudBackupVerificationState.Verified
        }
        device.scrollUntilText("Backup verified")
        assertEquals(assertionsBefore + 2, passkeyAssertionCount())
        assertEquals(
            "retrying verification must not create a new passkey",
            createsBefore,
            ScriptedPasskeyProvider.callCount(Invocation.CREATE),
        )

        // the provider holds one passkey, the one the restore discovered, so any other credential
        // would be a passkey a real phone might not have
        val retryCredentials = ScriptedPasskeyProvider.authenticatedCredentialIds().drop(authenticationsBefore)
        assertTrue(
            "verification must authenticate with the restored passkey, got $retryCredentials",
            retryCredentials.all { id -> id == ScriptedPasskeyProvider.knownCredentialIdHex() },
        )
    }

    // a wallet added while its upload cannot finish, so the detail offers Sync Now for it
    private fun restoreFixtureWithPendingWallet(): String {
        restoreFixtureAndOpenCloudBackupDetail()
        ScriptedCloudStorageAccess.rejectNextNewWalletUploadForQuota()

        val wallet =
            requireNotNull(importTestWalletOrNull(PENDING_WALLET)) {
                "a freshly reset app should not hold the pending test wallet"
            }
        waitUntil("the automatic upload of the new wallet should be rejected") {
            ScriptedCloudStorageAccess.rejectedNewWalletUploadCount() == 1
        }
        // the rejected upload must have settled, since Sync Now skips a wallet still uploading
        waitUntil("the detail should list the new wallet as needing sync after its upload failed") {
            CloudBackupManager.getInstance().detail?.needsSync?.any { item ->
                item.name == wallet.name && item.syncStatus != CloudBackupWalletStatus.UPLOADING
            } == true
        }
        device.scrollUntilText(SYNC_NOW)

        return wallet.name
    }

    private fun assertSyncedDetailShows(walletName: String) {
        waitUntil("the sync should finish and the detail refresh to show the wallet up to date") {
            val manager = CloudBackupManager.getInstance()
            val detail = manager.detail

            manager.syncState == CloudBackupSyncState.Idle &&
                manager.isDetailInventoryComplete &&
                detail?.needsSync?.isEmpty() == true &&
                detail.upToDate.any { it.name == walletName }
        }

        // the screen must show the refresh too: the wallet listed, and no Needs Sync section or
        // Sync Now anywhere in the detail, not only in the current viewport
        val texts = device.textsAcrossDetail()
        assertTrue("the refreshed detail should show $walletName, saw $texts", walletName in texts)
        assertFalse(
            "the refreshed detail should have no Needs Sync section, saw $texts",
            texts.any { text -> text.endsWith("Needs Sync") },
        )
        assertFalse("Sync Now should disappear once nothing needs sync, saw $texts", SYNC_NOW in texts)
    }

    private fun passkeyAssertionCount(): Int =
        ScriptedPasskeyProvider.callCount(Invocation.AUTHENTICATE) +
            ScriptedPasskeyProvider.callCount(Invocation.DISCOVER)

    private fun restoreFixtureAndOpenCloudBackupDetail() {
        ScriptedCloudStorageAccess.configureProductionFixtureRestore()
        launchFullApp()

        device.waitForText("Google Drive Backup Found")
        device.clickText("Restore with Passkey")
        device.waitForText("You're all set")
        device.waitForText("Your wallets have been restored.")
        device.clickText("Done")
        FullLaunchOnboardingRobot(device).acceptTermsAfterImport()
        assertTrue(
            "accepting terms should complete onboarding before opening Cloud Backup settings",
            device.wait(Until.gone(By.text("Terms & Conditions")), 10_000L),
        )

        // onboarding's last step selects the restored wallet and resets the route, which would
        // replace a settings route opened before it
        waitUntil("onboarding should finish on the restored wallet") {
            val app = AppManager.getInstance()
            val walletId = app.wallets.singleOrNull()?.id

            walletId != null &&
                app.router.default == Route.SelectedWallet(walletId) &&
                app.router.routes.isEmpty()
        }

        device.openSettings()
        val walletListsBeforeEntry = ScriptedCloudStorageAccess.walletListCount()
        device.openCloudBackup()

        // the detail held from the restore can already look complete, so wait for the refresh the
        // entry starts and for the restore's upload confirmation before changing anything
        waitUntil("expected the entry refresh and upload confirmation to settle", timeoutMs = 60_000L) {
            val manager = CloudBackupManager.getInstance()

            ScriptedCloudStorageAccess.walletListCount() > walletListsBeforeEntry &&
                manager.isDetailInventoryComplete &&
                !manager.isDetailInventoryChecking &&
                !manager.hasPendingUploadVerification &&
                manager.detail?.needsSync?.isEmpty() == true
        }
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
        timeoutMs: Long = 20_000L,
    ): UiObject2 =
        wait(Until.findObject(By.text(value)), timeoutMs)
            ?: error("Timed out waiting for text \"$value\"")

    private fun UiDevice.waitForTextContaining(
        value: String,
        timeoutMs: Long = 20_000L,
    ): UiObject2 =
        wait(Until.findObject(By.textContains(value)), timeoutMs)
            ?: error("Timed out waiting for text containing \"$value\"")

    // collects every text the detail shows, swiping to its end and back to its top
    private fun UiDevice.textsAcrossDetail(): Set<String> {
        val texts = mutableSetOf<String>()

        repeat(DETAIL_SCAN_SWIPES * 2) { swipe ->
            try {
                findObjects(By.textStartsWith("")).mapNotNullTo(texts) { node -> node.text }
            } catch (_: StaleObjectException) {
                // the next swipe reads the moved nodes again
            }

            val upward = swipe >= DETAIL_SCAN_SWIPES
            val startY = if (upward) displayHeight / 4 else displayHeight * 3 / 4
            val endY = if (upward) displayHeight * 3 / 4 else displayHeight / 4
            swipe(displayWidth / 2, startY, displayWidth / 2, endY, 20)
            Thread.sleep(250)
        }

        findObjects(By.textStartsWith("")).mapNotNullTo(texts) { node -> node.text }
        return texts
    }

    // scrolls down, then back up, since the detail may already be scrolled past the text
    private fun UiDevice.scrollUntilText(
        value: String,
        timeoutMs: Long = 20_000L,
    ): UiObject2 {
        val deadline = System.currentTimeMillis() + timeoutMs
        var swipes = 0

        while (System.currentTimeMillis() < deadline) {
            findObject(By.text(value))?.let { return it }
            val upward = (swipes / 4) % 2 == 1
            val startY = if (upward) displayHeight / 4 else displayHeight * 3 / 4
            val endY = if (upward) displayHeight * 3 / 4 else displayHeight / 4
            swipe(displayWidth / 2, startY, displayWidth / 2, endY, 20)
            swipes += 1
            Thread.sleep(250)
        }

        error("Timed out scrolling to text \"$value\"")
    }

    private fun UiDevice.clickText(value: String) {
        val deadline = System.currentTimeMillis() + 20_000L

        while (System.currentTimeMillis() < deadline) {
            try {
                var clickable: UiObject2? = scrollUntilText(value)
                while (clickable != null && !clickable.isClickable) {
                    clickable = clickable.parent
                }

                requireNotNull(clickable) { "expected a clickable ancestor for $value" }.click()
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
            "detail=${manager.detail}, sync=${manager.syncState}, verification=${manager.verificationState}, " +
                "uploads=${ScriptedCloudStorageAccess.acceptedNewWalletUploadCount()}, " +
                "rejected=${ScriptedCloudStorageAccess.rejectedNewWalletUploadCount()}, " +
                "create=${ScriptedPasskeyProvider.callCount(Invocation.CREATE)}, " +
                "authenticate=${ScriptedPasskeyProvider.callCount(Invocation.AUTHENTICATE)}, " +
                "discover=${ScriptedPasskeyProvider.callCount(Invocation.DISCOVER)}"
        assertTrue("$message; $diagnostics", condition())
    }
}
