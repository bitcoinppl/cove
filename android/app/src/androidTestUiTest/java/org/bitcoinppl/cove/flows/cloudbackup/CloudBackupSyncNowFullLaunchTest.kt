package org.bitcoinppl.cove.flows.cloudbackup

import android.view.View
import android.view.ViewGroup
import android.view.inspector.WindowInspector
import androidx.compose.ui.node.RootForTest
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.semantics.getAllSemanticsNodes
import androidx.compose.ui.semantics.getOrNull
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.filters.LargeTest
import androidx.test.platform.app.InstrumentationRegistry
import androidx.test.uiautomator.By
import androidx.test.uiautomator.BySelector
import androidx.test.uiautomator.Direction
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
        const val DETAIL_SCAN_MAX_SCROLLS = 20
        const val FAILURE_REOPEN_ATTEMPTS = 6

        // every label the wallet row's status badge can show
        val STATUS_BADGE_LABELS =
            listOf("Dirty", "Syncing", "Confirmed", "Failed", "Not on device", "Unsupported", "Unknown")

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
        ScriptedPasskeyProvider.configureResults(Invocation.AUTHENTICATE, Result.NOT_COMPLETED, Result.SUCCESS)
        ScriptedPasskeyProvider.configureResults(Invocation.DISCOVER, Result.NOT_COMPLETED, Result.SUCCESS)

        device.clickText("Verify Again")

        device.scrollUntilText("Cloud Backup Not Verified")
        waitUntil("cancelling the passkey should leave verification not completed") {
            CloudBackupManager.getInstance().verificationState == CloudBackupVerificationState.NotCompleted
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
        // Sync Now skips a wallet still uploading, so wait for the detail to show the rejected
        // upload's persisted failure; the failure itself requests no detail refresh, so a refresh
        // that ran before it keeps showing the dirty wallet until the detail is opened again
        val showsFailure = {
            CloudBackupManager.getInstance().walletsSection?.needsSync?.any { row ->
                row.wallet.name == wallet.name && row.wallet.syncStatus == CloudBackupWalletStatus.FAILED
            } == true
        }
        var reopens = 0

        while (!pollUntil(5_000L, showsFailure) && reopens < FAILURE_REOPEN_ATTEMPTS) {
            device.openSettings()
            device.openCloudBackup()
            reopens += 1
        }

        waitUntil("the detail should list the new wallet as needing sync after its upload failed", 1_000L) {
            showsFailure()
        }
        device.scrollUntilText(SYNC_NOW)

        return wallet.name
    }

    private fun assertSyncedDetailShows(walletName: String) {
        waitUntil("the sync should finish and the detail refresh to show the wallet up to date") {
            val manager = CloudBackupManager.getInstance()
            val wallets = manager.walletsSection

            manager.syncState == CloudBackupSyncState.Idle &&
                manager.isInventoryComplete &&
                wallets?.needsSync?.isEmpty() == true &&
                wallets.upToDate.any { it.wallet.name == walletName }
        }

        // the screen must show the refresh too: no Needs Sync section or Sync Now anywhere in the
        // detail, not only in the current viewport; a wallet row merges its texts into one node, so
        // these match within texts
        val texts = device.textsAcrossDetail()
        assertTrue("the refreshed detail should show $walletName, saw $texts", texts.any { walletName in it })
        assertFalse(
            "the refreshed detail should have no Needs Sync section, saw $texts",
            texts.any { text -> "Needs Sync" in text },
        )
        assertFalse(
            "Sync Now should disappear once nothing needs sync, saw $texts",
            texts.any { text -> SYNC_NOW in text },
        )

        val badges = device.walletRowBadges(walletName)
        assertEquals("the refreshed wallet's row should show only the Confirmed badge", setOf("Confirmed"), badges)
    }

    // the status badges in the wallet's own row, read from compose's merged semantics tree: the row
    // merges its texts into one node, so only that node's own texts are read and another wallet's
    // badge can't count. uiautomator's view of a merged row doesn't list the badge text
    private fun UiDevice.walletRowBadges(walletName: String): Set<String> {
        scrollUntil(By.textContains(walletName))

        val rows = mutableListOf<List<String>>()
        InstrumentationRegistry.getInstrumentation().runOnMainSync {
            rows +=
                WindowInspector
                    .getGlobalWindowViews()
                    .flatMap { it.composeRoots() }
                    .flatMap { root -> root.semanticsOwner.getAllSemanticsNodes(mergingEnabled = true) }
                    .filter { node -> node.config.isMergingSemanticsOfDescendants }
                    .map { node -> node.config.getOrNull(SemanticsProperties.Text).orEmpty().map { it.text } }
                    .filter { texts -> walletName in texts }
        }

        check(rows.size == 1) { "expected one row for $walletName, saw $rows" }
        val texts = rows.single()
        val badges = STATUS_BADGE_LABELS.filterTo(mutableSetOf()) { label -> label in texts }
        check(badges.isNotEmpty()) { "the row for $walletName shows no status badge, saw $texts" }

        return badges
    }

    private fun View.composeRoots(): List<RootForTest> =
        when (this) {
            is RootForTest -> listOf(this)
            is ViewGroup -> (0 until childCount).flatMap { getChildAt(it).composeRoots() }
            else -> emptyList()
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
        val app = AppManager.getInstance()
        val finished =
            pollUntil(30_000L) {
                app.router.default is Route.SelectedWallet && app.router.routes.isEmpty()
            }
        assertTrue(
            "onboarding should finish on a selected wallet; default=${app.router.default}, " +
                "routes=${app.router.routes}, wallets=${app.wallets.map { it.name }}",
            finished,
        )

        device.openSettings()
        val walletListsBeforeEntry = ScriptedCloudStorageAccess.walletListCount()
        device.openCloudBackup()

        // the detail held from the restore can already look complete, so wait for the refresh the
        // entry starts and for the restore's upload confirmation before changing anything
        waitUntil("expected the entry refresh and upload confirmation to settle", timeoutMs = 60_000L) {
            val manager = CloudBackupManager.getInstance()

            ScriptedCloudStorageAccess.walletListCount() > walletListsBeforeEntry &&
                manager.isInventoryComplete &&
                !manager.isInventoryChecking &&
                !manager.hasPendingUploadVerification &&
                manager.walletsSection?.needsSync?.isEmpty() == true
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

    // collects every text the detail shows, scrolling its container until the container reports no
    // more scroll at its end and again at its top, and fails rather than return a partial scan
    private fun UiDevice.textsAcrossDetail(): Set<String> {
        val texts = mutableSetOf<String>()

        listOf(Direction.DOWN, Direction.UP).forEach { direction ->
            var scrolls = 0
            var more = true

            while (more) {
                texts += visibleTexts()
                check(scrolls < DETAIL_SCAN_MAX_SCROLLS) {
                    "scanning the detail never reached its ${direction.name.lowercase()} end, saw $texts"
                }

                more = scrollDetail(direction)
                scrolls += 1
            }

            texts += visibleTexts()
        }

        return texts
    }

    // scrolls the detail's container, finding it again when a recomposition replaces it; returns
    // false once the container cannot scroll further that way
    private fun UiDevice.scrollDetail(direction: Direction): Boolean {
        repeat(5) {
            try {
                // compose marks a list scrollable only while it can scroll, so a detail that fits on
                // screen has no scrollable node and the visible texts are all of it
                val container = findObject(By.scrollable(true)) ?: return false

                return container.scroll(direction, 0.8f)
            } catch (_: StaleObjectException) {
                Thread.sleep(100)
            }
        }

        error("the detail's container kept changing while it was scrolled")
    }

    // reads the same viewport again when the tree moves under the read
    private fun UiDevice.visibleTexts(): List<String> {
        repeat(5) {
            try {
                return findObjects(By.textStartsWith("")).mapNotNull { node -> node.text }
            } catch (_: StaleObjectException) {
                Thread.sleep(100)
            }
        }

        error("the detail kept changing while its texts were read")
    }

    // scrolls down, then back up, since the detail may already be scrolled past the text
    private fun UiDevice.scrollUntilText(
        value: String,
        timeoutMs: Long = 20_000L,
    ): UiObject2 = scrollUntil(By.text(value), timeoutMs)

    private fun UiDevice.scrollUntil(
        selector: BySelector,
        timeoutMs: Long = 20_000L,
    ): UiObject2 {
        val deadline = System.currentTimeMillis() + timeoutMs
        var swipes = 0

        while (System.currentTimeMillis() < deadline) {
            findObject(selector)?.let { return it }
            val upward = (swipes / 4) % 2 == 1
            val startY = if (upward) displayHeight / 4 else displayHeight * 3 / 4
            val endY = if (upward) displayHeight * 3 / 4 else displayHeight / 4
            swipe(displayWidth / 2, startY, displayWidth / 2, endY, 20)
            swipes += 1
            Thread.sleep(250)
        }

        error("Timed out scrolling to $selector")
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

    private fun pollUntil(
        timeoutMs: Long,
        condition: () -> Boolean,
    ): Boolean {
        val deadline = System.currentTimeMillis() + timeoutMs

        while (System.currentTimeMillis() < deadline) {
            if (condition()) return true

            Thread.sleep(100)
        }

        return condition()
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
            "screen=${manager.describeScreen()}, sync=${manager.syncState}, " +
                "uploads=${ScriptedCloudStorageAccess.acceptedNewWalletUploadCount()}, " +
                "rejected=${ScriptedCloudStorageAccess.rejectedNewWalletUploadCount()}, " +
                "create=${ScriptedPasskeyProvider.callCount(Invocation.CREATE)}, " +
                "authenticate=${ScriptedPasskeyProvider.callCount(Invocation.AUTHENTICATE)}, " +
                "discover=${ScriptedPasskeyProvider.callCount(Invocation.DISCOVER)}"
        assertTrue("$message; $diagnostics", condition())
    }
}
