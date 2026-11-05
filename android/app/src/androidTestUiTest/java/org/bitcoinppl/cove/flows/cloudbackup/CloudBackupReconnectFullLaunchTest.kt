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
import org.bitcoinppl.cove.test.fullLaunchDevice
import org.bitcoinppl.cove.test.launchFullApp
import org.bitcoinppl.cove.testconfig.ScriptedCloudBackupFixture
import org.bitcoinppl.cove.testconfig.ScriptedCloudStorageAccess
import org.bitcoinppl.cove.testconfig.ScriptedConnectivityAccess
import org.bitcoinppl.cove.testconfig.ScriptedPasskeyProvider
import org.bitcoinppl.cove.testconfig.ScriptedPasskeyProvider.Invocation
import org.bitcoinppl.cove_core.CloudBackupRestoreAllState
import org.bitcoinppl.cove_core.CloudBackupVerificationState
import org.bitcoinppl.cove_core.CloudOnlyState
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
class CloudBackupReconnectFullLaunchTest {
    private companion object {
        const val OFFLINE_DETAIL_MESSAGE =
            "This device is offline. Connect to the internet, then check Google Drive again."
        const val RESTORE_ALL_TITLE = "Restore All (2)"
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
    fun reconnectAloneCompletesInventoryWithoutPasskeyPrompt() {
        restoreFixtureAndOpenCloudBackupDetail()
        val expectedRecordIds = cloudOnlyRecordIds()
        val passkeyCallsBeforeOffline = passkeyCallCounts()

        // a user who reopens Cloud Backup while offline gets the failed refresh
        ScriptedConnectivityAccess.setConnected(false)
        device.openSettings()
        device.openCloudBackup()

        waitUntil("reopening the detail offline should fail the refresh and keep the known rows") {
            val manager = CloudBackupManager.getInstance()

            manager.detailError == OFFLINE_DETAIL_MESSAGE &&
                !manager.isDetailInventoryComplete &&
                manager.restoreAllState == CloudBackupRestoreAllState.StartDisabled(2u) &&
                cloudOnlyRecordIds() == expectedRecordIds
        }
        device.scrollUntilText(OFFLINE_DETAIL_MESSAGE)
        device.scrollUntilText("Fixture Wallet Two")
        device.scrollUntilText("Fixture Wallet Three")
        assertFalse(
            "Restore All must stay disabled while the inventory is incomplete",
            device.scrollUntilText(RESTORE_ALL_TITLE).clickableAncestor().isEnabled,
        )

        // the reconnect itself must refresh the open detail, so the test starts no refresh; the
        // provider counts only listings it served, so a later count shows the inventory was read
        // again rather than the offline snapshot marked complete
        val walletListsBeforeReconnect = ScriptedCloudStorageAccess.walletListCount()
        ScriptedConnectivityAccess.setConnected(true)

        waitUntil("reconnecting should complete the inventory without another refresh request") {
            val manager = CloudBackupManager.getInstance()

            ScriptedCloudStorageAccess.walletListCount() > walletListsBeforeReconnect &&
                manager.detailError == null &&
                manager.isDetailInventoryComplete &&
                manager.restoreAllState == CloudBackupRestoreAllState.StartAvailable(2u) &&
                cloudOnlyRecordIds() == expectedRecordIds
        }
        assertTrue(
            "the offline message should clear once the inventory completes",
            device.wait(Until.gone(By.text(OFFLINE_DETAIL_MESSAGE)), 10_000L),
        )
        device.scrollUntilText("Fixture Wallet Two")
        device.scrollUntilText("Fixture Wallet Three")
        assertTrue(
            "Restore All should be available again after reconnecting",
            device.scrollUntilText(RESTORE_ALL_TITLE).clickableAncestor().isEnabled,
        )
        assertEquals(
            "refreshing the inventory must neither create nor ask for a passkey",
            passkeyCallsBeforeOffline,
            passkeyCallCounts(),
        )
        assertEquals(0, ScriptedPasskeyProvider.callCount(Invocation.CREATE))
    }

    private fun restoreFixtureAndOpenCloudBackupDetail() {
        ScriptedCloudStorageAccess.configureProductionFixtureRestore()
        launchFullApp()

        device.waitForText("Google Drive Backup Found")
        device.clickClickableAncestor(device.waitForText("Restore with Passkey"))
        device.waitForText("You're all set")
        device.waitForText("Your wallets have been restored.")
        ScriptedCloudStorageAccess.exposeAllProductionFixtureWallets()
        device.clickClickableAncestor(device.waitForText("Done"))
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
        // entry starts, the restore's upload confirmation and its verification, whose cloud-only
        // reload would otherwise race the network drop
        waitUntil(
            "expected the entry refresh to settle with two fixture wallets and Restore All",
            timeoutMs = 60_000L,
        ) {
            val manager = CloudBackupManager.getInstance()

            ScriptedCloudStorageAccess.walletListCount() > walletListsBeforeEntry &&
                manager.isDetailInventoryComplete &&
                !manager.isDetailInventoryChecking &&
                !manager.hasPendingUploadVerification &&
                manager.verificationState is CloudBackupVerificationState.Verified &&
                manager.restoreAllState == CloudBackupRestoreAllState.StartAvailable(2u) &&
                cloudOnlyRecordIds() ==
                listOf(
                    ScriptedCloudBackupFixture.WALLET_TWO_RECORD_ID,
                    ScriptedCloudBackupFixture.WALLET_THREE_RECORD_ID,
                )
        }
    }

    private fun passkeyCallCounts(): Map<Invocation, Int> =
        Invocation.entries.associateWith(ScriptedPasskeyProvider::callCount)

    private fun cloudOnlyRecordIds(): List<String> =
        ((CloudBackupManager.getInstance().cloudOnly as? CloudOnlyState.Loaded)?.wallets)
            .orEmpty()
            .map { wallet -> wallet.recordId }

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

    private fun UiObject2.clickableAncestor(): UiObject2 {
        var clickable: UiObject2? = this

        while (clickable != null && !clickable.isClickable) {
            clickable = clickable.parent
        }

        return requireNotNull(clickable) { "expected a clickable ancestor for $text" }
    }

    private fun UiDevice.clickClickableAncestor(node: UiObject2) {
        node.clickableAncestor().click()
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
            "detail=${manager.state.lifecycle}, detailError=${manager.detailError}, " +
                "verification=${manager.verificationState}, " +
                "restoreAll=${manager.restoreAllState}, cloudOnly=${cloudOnlyRecordIds()}, " +
                "passkeyCalls=${passkeyCallCounts()}, walletLists=${ScriptedCloudStorageAccess.walletListCount()}"
        assertTrue("$message; $diagnostics", condition())
    }
}
