package org.bitcoinppl.cove.flows.cloudbackup

import androidx.test.platform.app.InstrumentationRegistry
import androidx.test.uiautomator.By
import androidx.test.uiautomator.StaleObjectException
import androidx.test.uiautomator.UiDevice
import androidx.test.uiautomator.UiObject2
import androidx.test.uiautomator.Until
import org.bitcoinppl.cove.AppManager
import org.bitcoinppl.cove.cloudbackup.CloudBackupManager
import org.bitcoinppl.cove.cloudbackup.CloudBackupTags
import org.bitcoinppl.cove.cloudbackup.configured
import org.bitcoinppl.cove.test.FullLaunchOnboardingRobot
import org.bitcoinppl.cove.test.FullLaunchStartupRobot
import org.bitcoinppl.cove.test.launchFullApp
import org.bitcoinppl.cove.testconfig.ScriptedCloudBackupFixture
import org.bitcoinppl.cove.testconfig.ScriptedCloudStorageAccess
import org.bitcoinppl.cove.testconfig.ScriptedPasskeyProvider
import org.bitcoinppl.cove.testconfig.ScriptedPasskeyProvider.Invocation
import org.bitcoinppl.cove_core.BootstrapStep
import org.bitcoinppl.cove_core.CloudBackupDisableScreen
import org.bitcoinppl.cove_core.CloudBackupListedRecord
import org.bitcoinppl.cove_core.CloudBackupScreenAction
import org.bitcoinppl.cove_core.CloudBackupScreenBody
import org.bitcoinppl.cove_core.CloudBackupVerificationState
import org.bitcoinppl.cove_core.Route
import org.bitcoinppl.cove_core.SettingsRoute
import org.bitcoinppl.cove_core.bootstrapProgress
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue

/**
 * The Turn Off steps the full-launch Turn Off scenarios share, against the scripted cloud
 *
 * A fresh enable backs up only this device's wallets, so its Turn Off asks once; restoring Fixture
 * Wallet One and then listing wallets two and three leaves those only in the cloud, so its Turn
 * Off asks twice. The review dialogs are separate windows without resource ids, so their buttons
 * are found by text, while the detail's controls are found by their tags
 */
internal class CloudBackupTurnOffRobot(
    private val device: UiDevice,
) {
    val manager: CloudBackupManager
        get() = CloudBackupManager.getInstance()

    // setup

    /**
     * Restores Fixture Wallet One in onboarding, then lists [listedAfterRestore] beside it, opens
     * Cloud Backup, verifies the backup with Verify Now and waits until the inventory has listed them
     */
    fun restoreWalletOneAndOpenDetail(
        listedAfterRestore: List<String> = NOT_ON_DEVICE_RECORD_IDS,
        resetData: Boolean = true,
    ) {
        ScriptedCloudStorageAccess.configureProductionFixtureRestore()
        launchApp(resetData)

        device.waitForText("Google Drive Backup Found")
        device.clickText("Restore with Passkey")
        device.waitForText("You're all set")
        ScriptedCloudStorageAccess.exposeProductionFixtureWallets(
            listOf(ScriptedCloudBackupFixture.WALLET_RECORD_ID) + listedAfterRestore,
        )
        device.clickText("Done")
        FullLaunchOnboardingRobot(device).acceptTermsAfterImport()
        assertTrue(
            "accepting terms should complete onboarding before opening Cloud Backup",
            device.wait(Until.gone(By.text("Terms & Conditions")), 10_000L),
        )

        awaitOnboardingRoute()
        openCloudBackup()
        verifyRestoredBackup()
        waitUntil("the inventory never listed the restored wallet's other records", timeoutMs = 60_000L) {
            manager.isInventoryComplete &&
                !manager.isInventoryChecking &&
                manager.verificationState is CloudBackupVerificationState.Verified &&
                listedRecordIds() == listedAfterRestore.toSet()
        }
        manager.assertNextActions("a settled backup should offer Turn Off", available = setOf(TURN_OFF))
    }

    /**
     * Verifies a restored backup the way a user does, with Verify Now on its card, which the
     * scripted provider answers with the passkey the restore found
     *
     * A restore doesn't count as verification, and the detail starts none on its own, so a
     * restored backup stays not verified until the user taps Verify Now
     */
    fun verifyRestoredBackup() {
        waitUntil("a restored backup should wait for Verify Now", timeoutMs = 60_000L) {
            manager.isInventoryComplete &&
                manager.verificationState == CloudBackupVerificationState.NotVerified
        }
        manager.assertNextActions(
            "a restored backup should offer Verify",
            available = setOf(CloudBackupScreenAction.VERIFY),
        )
        clickText("Verify Now", scroll = true)
    }

    /** Enables Cloud Backup from onboarding and opens it once the fresh backup is confirmed */
    fun freshEnableAndOpenDetail(persistent: Boolean = false) {
        if (persistent) {
            ScriptedCloudStorageAccess.configurePersistentFreshEnableWithDelayedVisibility(resetStoredState = true)

            // a persistent enable is a staged test's first stage, and its next stage checks it runs
            // in a new process against this record
            assertTrue(
                "expected the first process of a staged Turn Off scenario",
                ScriptedCloudStorageAccess.recordProcessAndCheckRestart(expectedRestart = false),
            )
        } else {
            ScriptedCloudStorageAccess.configureFreshEnableWithDelayedVisibility()
        }

        launchApp(resetData = true)
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

        ScriptedCloudStorageAccess.releaseVisibility()
        waitUntil("the fresh backup never confirmed", timeoutMs = 60_000L) { !manager.hasPendingUploadVerification }

        device.clickText("Continue")
        onboarding.continueFromBackupWallet().acceptTermsAfterImport()
        assertTrue(
            "accepting terms should complete onboarding before opening Cloud Backup",
            device.wait(Until.gone(By.text("Terms & Conditions")), 10_000L),
        )
        assertFalse("the enable wrote no backup", ScriptedCloudStorageAccess.freshEnableNamespaces().isEmpty())

        awaitOnboardingRoute()
        openCloudBackup()
        waitUntil("the inventory never loaded") { manager.isInventoryComplete && !manager.isInventoryChecking }
        manager.assertNextActions("a confirmed backup should offer Turn Off", available = setOf(TURN_OFF))
    }

    /** Starts the app, resetting its data unless [resetData] is false, and waits for bootstrap */
    fun launchApp(resetData: Boolean) {
        // the reset only runs in a fresh activity, which the test orchestrator's process per test gives
        launchFullApp(resetData = resetData)

        // the activity starts before bootstrap finishes, and AppManager is unavailable until it does
        val deadline = System.currentTimeMillis() + 30_000L

        while (bootstrapProgress() != BootstrapStep.COMPLETE && System.currentTimeMillis() < deadline) {
            Thread.sleep(100)
        }

        assertEquals("the app should finish bootstrapping after launch", BootstrapStep.COMPLETE, bootstrapProgress())
    }

    /**
     * Opens the Cloud Backup detail as the only route, as the other Cloud Backup full-launch tests
     * do; a push on top of a just-reset Settings route can be replaced by that reset a moment later
     */
    fun openCloudBackup() {
        InstrumentationRegistry.getInstrumentation().runOnMainSync {
            AppManager.getInstance().resetRoute(Route.Settings(SettingsRoute.CloudBackup))
        }

        waitUntil("the Cloud Backup detail never opened") {
            AppManager.getInstance().currentRoute == Route.Settings(SettingsRoute.CloudBackup)
        }
        device.waitForText("Cloud Backup")
    }

    fun openSettings() {
        InstrumentationRegistry.getInstrumentation().runOnMainSync {
            AppManager.getInstance().resetRoute(Route.Settings(SettingsRoute.Main))
        }

        device.waitForText("Security")
    }

    // finishing onboarding resets the route a moment later, which would replace a route opened before it
    private fun awaitOnboardingRoute() {
        waitUntil("onboarding should finish on a wallet") {
            val router = AppManager.getInstance().router
            router.default is Route.SelectedWallet && router.routes.isEmpty()
        }
    }

    // Turn Off

    /** Taps the detail's Turn Off, which must be enabled */
    fun tapTurnOff() {
        manager.assertNextActions("Turn Off is not available", available = setOf(TURN_OFF))
        device.clickTagged(CloudBackupTags.action(TURN_OFF), scroll = true)
    }

    /** Waits for the first of two dialogs, which Rust opens when Turn Off deletes something unrecoverable */
    fun waitForFirstDialog(): UiObject2 {
        waitUntil("the Turn Off review never showed its first dialog", timeoutMs = 20_000L) {
            device.hasObject(By.text(FIRST_DIALOG_TITLE))
        }

        return device.findObject(By.text(FIRST_DIALOG_TITLE))
    }

    /** Waits for the dialog that confirms the delete, the only one when Turn Off asks once */
    fun waitForFinalDialog() {
        waitUntil("the Turn Off review never asked to confirm", timeoutMs = 20_000L) {
            device.hasObject(By.text(CONFIRM))
        }
    }

    /** Confirms a review that asks once */
    fun confirmOnce() {
        waitForFinalDialog()
        assertFalse("a review that should ask once asked twice", device.hasObject(By.text(FIRST_DIALOG_TITLE)))
        device.clickText(CONFIRM)
    }

    /** Continues past the first dialog and confirms the second */
    fun confirmTwice() {
        waitForFirstDialog()
        assertFalse("the first dialog already offered the delete", device.hasObject(By.text(CONFIRM)))
        device.clickText(CONTINUE)
        waitForFinalDialog()
        assertTrue("the second dialog doesn't name the delete", device.hasObject(By.text(TWO_STEP_FINAL_TITLE)))
        device.clickText(CONFIRM)
    }

    /** Turn Off finished: Cloud Backup is off and offers Enable */
    fun waitForTurnedOff() {
        manager.assertNextActions(
            "Turn Off should finish with Cloud Backup off",
            body = CloudBackupScreenBody.Disabled::class,
            available = setOf(CloudBackupScreenAction.ENABLE),
            timeoutMs = 60_000L,
        )
    }

    /**
     * Turn Off finished on screen: the detail went back to Settings, whose Cloud Backup
     * row says Off, and that row opens an Enable the user can tap once the setup acknowledgements
     * are checked
     */
    fun assertBackOnSettingsWithEnable() {
        waitUntil("Turn Off should go back to Settings") {
            AppManager.getInstance().currentRoute == Route.Settings(SettingsRoute.Main)
        }
        device.clickText(OFF_ROW)
        waitUntil("the Off row should open Cloud Backup") {
            AppManager.getInstance().currentRoute == Route.Settings(SettingsRoute.CloudBackup)
        }

        val enable = By.res(CloudBackupTags.action(CloudBackupScreenAction.ENABLE))
        assertTrue("Cloud Backup draws no Enable", device.wait(Until.hasObject(enable), 10_000L))

        FullLaunchOnboardingRobot(device).acceptCloudBackupAcknowledgements()
        waitUntil("Enable should be enabled once the acknowledgements are checked") {
            try {
                device.findObject(enable)?.isEnabled == true
            } catch (_: StaleObjectException) {
                false
            }
        }
    }

    /** Turn Off is offered again, its dialogs are gone, and no delete was asked for since [deletesBefore] */
    fun assertTurnOffIdle(
        after: String,
        deletesBefore: Int,
    ) {
        waitUntil("$after left a Turn Off dialog up", timeoutMs = 10_000L) {
            !device.hasObject(By.text(FIRST_DIALOG_TITLE)) && !device.hasObject(By.text(CONFIRM))
        }
        manager.assertNextActions("$after should offer Turn Off again", available = setOf(TURN_OFF))
        waitUntil("$after should leave Turn Off idle") { manager.configured?.disable == CloudBackupDisableScreen.Idle }
        assertEquals("$after deleted a backup", deletesBefore, ScriptedCloudStorageAccess.namespaceDeleteCount())
    }

    // reads

    /** The record ids of the listed rows, Not on This Device and Extra Copies alike */
    fun listedRecordIds(): Set<String> =
        (manager.notOnDeviceRows + manager.extraCopyRows).map { it.recordId }.toSet()

    fun extraCopyRecordIds(): Set<String> = manager.extraCopyRows.map { it.recordId }.toSet()

    fun uncheckedRowCount(): Int = manager.notOnDeviceRows.count { it.record is CloudBackupListedRecord.Unchecked }

    fun passkeyCalls(): List<Int> = Invocation.entries.map(ScriptedPasskeyProvider::callCount)

    // device steps

    fun waitForText(value: String): UiObject2 = device.waitForText(value)

    /**
     * Waits for [value] on a scrolling screen, swiping toward the top and then back down while it
     * isn't on screen, since UiAutomator finds only what is visible and a section can grow above
     * the control the test last scrolled to
     */
    fun scrollToText(
        value: String,
        timeoutMs: Long = 20_000L,
    ): UiObject2 {
        val deadline = System.currentTimeMillis() + timeoutMs
        var towardTop = true
        var swipes = 0

        while (System.currentTimeMillis() < deadline) {
            device.findObject(By.text(value))?.let { return it }

            // a finger moving down shows what is above
            val (fromY, toY) =
                if (towardTop) device.displayHeight / 4 to device.displayHeight * 3 / 4
                else device.displayHeight * 3 / 4 to device.displayHeight / 4
            device.swipe(device.displayWidth / 2, fromY, device.displayWidth / 2, toY, 20)
            device.waitForIdle()

            swipes += 1
            if (swipes % SWIPES_PER_PASS == 0) towardTop = !towardTop
        }

        error("Timed out scrolling to text \"$value\"; ${manager.describeScreen()}")
    }

    fun clickText(
        value: String,
        scroll: Boolean = false,
    ) = device.clickText(value, scroll)

    fun clickTagged(
        tag: String,
        scroll: Boolean = false,
    ) = device.clickTagged(tag, scroll)

    fun waitUntil(
        message: String,
        timeoutMs: Long = 30_000L,
        condition: () -> Boolean,
    ) {
        val deadline = System.currentTimeMillis() + timeoutMs

        while (System.currentTimeMillis() < deadline) {
            if (condition()) return

            Thread.sleep(100)
        }

        val diagnostics =
            "${manager.describeScreen()}, actions=${manager.describeActions()}, " +
                "listed=${listedRecordIds()}, route=${AppManager.getInstance().currentRoute}, " +
                "namespaceDeletes=${ScriptedCloudStorageAccess.namespaceDeleteCount()}, " +
                "deletedWallets=${ScriptedCloudStorageAccess.deletedWalletRecordIds()}"

        assertTrue("$message; $diagnostics", condition())
    }

    private fun UiDevice.waitForText(
        value: String,
        timeoutMs: Long = 20_000L,
    ): UiObject2 =
        wait(Until.findObject(By.text(value)), timeoutMs)
            ?: error("Timed out waiting for text \"$value\"; ${manager.describeScreen()}")

    private fun UiDevice.clickText(
        value: String,
        scroll: Boolean = false,
        timeoutMs: Long = 20_000L,
    ) = clickFirst("text \"$value\"", scroll, timeoutMs) { findObjects(By.text(value)) }

    private fun UiDevice.clickTagged(
        tag: String,
        scroll: Boolean = false,
        timeoutMs: Long = 20_000L,
    ) = clickFirst("tag \"$tag\"", scroll, timeoutMs) { findObjects(By.res(tag)) }

    // clicks the first match that sits in something clickable, since a text can also appear as a
    // plain label, scrolling between looks when [scroll] is set
    private fun UiDevice.clickFirst(
        description: String,
        scroll: Boolean,
        timeoutMs: Long,
        find: () -> List<UiObject2>,
    ) {
        val deadline = System.currentTimeMillis() + timeoutMs

        while (System.currentTimeMillis() < deadline) {
            val clickable =
                try {
                    find().firstNotNullOfOrNull { node -> generateSequence(node) { it.parent }.firstOrNull { it.isClickable } }
                } catch (_: StaleObjectException) {
                    // the tree moved under the read, so look again
                    null
                }

            if (clickable != null) {
                clickable.click()
                return
            }

            if (scroll) {
                swipe(displayWidth / 2, displayHeight * 3 / 4, displayWidth / 2, displayHeight / 4, 20)

                // a tap while the list still flings only stops it, so the next tap must wait for it to settle
                waitForIdle()
            }

            Thread.sleep(100)
        }

        error(
            "Timed out clicking $description; route=${AppManager.getInstance().currentRoute}, " +
                "actions=${manager.describeActions()}, ${manager.describeScreen()}",
        )
    }

    companion object {
        val TURN_OFF = CloudBackupScreenAction.TURN_OFF

        const val FIRST_DIALOG_TITLE = "Before You Turn Off Cloud Backup"
        const val TWO_STEP_FINAL_TITLE = "Delete These Backups and Turn Off Cloud Backup?"
        const val CONFIRM = "Turn Off and Delete Backups"
        const val CONTINUE = "Continue"
        const val CANCEL = "Cancel"
        const val RESTORE_FIRST = "Restore Them First"
        const val VERIFY_FIRST = "Verify First"
        const val TRY_AGAIN = "Try Again"
        const val TURN_OFF_INCOMPLETE_ROW = "Turn off didn't finish — open to finish"
        const val OFF_ROW = "Off"

        // enough swipes to cross the detail screen from one end to the other
        const val SWIPES_PER_PASS = 6

        // wallets two and three, only in the cloud once Fixture Wallet One alone is restored
        val NOT_ON_DEVICE_RECORD_IDS =
            listOf(ScriptedCloudBackupFixture.WALLET_TWO_RECORD_ID, ScriptedCloudBackupFixture.WALLET_THREE_RECORD_ID)

        // a second backup of Fixture Wallet One, an extra copy once wallet one is on this device
        val EXTRA_COPY_RECORD_IDS = listOf(ScriptedCloudBackupFixture.DUPLICATE_WALLET_RECORD_ID)

        // the master key and the key that encrypts it, the local entries Cove reads backups with
        val LOCAL_MASTER_KEY_ENTRIES = listOf("cspp::v1::master_key", "cspp::v1::master_key_encryption_key_and_nonce")
    }
}
