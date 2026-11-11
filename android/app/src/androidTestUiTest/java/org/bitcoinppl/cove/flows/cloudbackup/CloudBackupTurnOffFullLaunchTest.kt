package org.bitcoinppl.cove.flows.cloudbackup

import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.filters.LargeTest
import androidx.test.platform.app.InstrumentationRegistry
import androidx.test.uiautomator.By
import androidx.test.uiautomator.UiDevice
import androidx.test.uiautomator.Until
import org.bitcoinppl.cove.AppManager
import org.bitcoinppl.cove.AuthManager
import org.bitcoinppl.cove.cloudbackup.CloudBackupTags
import org.bitcoinppl.cove.cloudbackup.caption
import org.bitcoinppl.cove.cloudbackup.configured
import org.bitcoinppl.cove.flows.cloudbackup.CloudBackupTurnOffRobot.Companion.CANCEL
import org.bitcoinppl.cove.flows.cloudbackup.CloudBackupTurnOffRobot.Companion.CONFIRM
import org.bitcoinppl.cove.flows.cloudbackup.CloudBackupTurnOffRobot.Companion.CONTINUE
import org.bitcoinppl.cove.flows.cloudbackup.CloudBackupTurnOffRobot.Companion.EXTRA_COPY_RECORD_IDS
import org.bitcoinppl.cove.flows.cloudbackup.CloudBackupTurnOffRobot.Companion.FIRST_DIALOG_TITLE
import org.bitcoinppl.cove.flows.cloudbackup.CloudBackupTurnOffRobot.Companion.NOT_ON_DEVICE_RECORD_IDS
import org.bitcoinppl.cove.flows.cloudbackup.CloudBackupTurnOffRobot.Companion.RESTORE_FIRST
import org.bitcoinppl.cove.flows.cloudbackup.CloudBackupTurnOffRobot.Companion.TRY_AGAIN
import org.bitcoinppl.cove.flows.cloudbackup.CloudBackupTurnOffRobot.Companion.TURN_OFF
import org.bitcoinppl.cove.flows.cloudbackup.CloudBackupTurnOffRobot.Companion.TURN_OFF_INCOMPLETE_ROW
import org.bitcoinppl.cove.test.FullLaunchTestRule
import org.bitcoinppl.cove.test.ManualFullLaunchTest
import org.bitcoinppl.cove.test.fullLaunchDevice
import org.bitcoinppl.cove.test.restoreNoAppLock
import org.bitcoinppl.cove.testconfig.ScriptedCloudBackupFixture
import org.bitcoinppl.cove.testconfig.ScriptedCloudStorageAccess
import org.bitcoinppl.cove.testconfig.ScriptedConnectivityAccess
import org.bitcoinppl.cove.testconfig.ScriptedPasskeyProvider
import org.bitcoinppl.cove.testconfig.ScriptedPasskeyProvider.Invocation
import org.bitcoinppl.cove.testconfig.ScriptedPasskeyProvider.Result
import org.bitcoinppl.cove_core.AppLockReason
import org.bitcoinppl.cove_core.AuthManagerAction
import org.bitcoinppl.cove_core.CloudBackupActionAvailability
import org.bitcoinppl.cove_core.CloudBackupDisableScreen
import org.bitcoinppl.cove_core.CloudBackupManagerAction
import org.bitcoinppl.cove_core.CloudBackupScreenAction
import org.bitcoinppl.cove_core.CloudBackupScreenBody
import org.bitcoinppl.cove_core.CloudBackupSettingsRowStatus
import org.bitcoinppl.cove_core.CloudBackupVerificationState
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

/**
 * Turns Cloud Backup off through the review Rust builds from the inventory, in one process
 *
 * The scenarios that need a relaunch are in [CloudBackupTurnOffRelaunchFullLaunchTest]. Each
 * scenario asserts what Rust offers next with `assertNextActions`, not only what is drawn
 */
@ManualFullLaunchTest
@LargeTest
@RunWith(AndroidJUnit4::class)
class CloudBackupTurnOffFullLaunchTest {
    private lateinit var device: UiDevice
    private lateinit var robot: CloudBackupTurnOffRobot

    @get:Rule
    val fullLaunch = FullLaunchTestRule()

    @Before
    fun prepareDevice() {
        ScriptedPasskeyProvider.reset()
        ScriptedConnectivityAccess.reset()
        device = fullLaunchDevice()
        robot = CloudBackupTurnOffRobot(device)
    }

    @After
    fun cleanUp() {
        // a failed assertion must not leave the provider held, the network off or a PIN set
        ScriptedCloudStorageAccess.releaseHeldWalletDelete()
        ScriptedCloudStorageAccess.releaseHeldNamespaceDelete()
        ScriptedCloudStorageAccess.releaseVisibility()
        ScriptedConnectivityAccess.reset()
        restoreNoAppLock()
    }

    @Test
    fun extraCopiesOnlyAskOnceAndTurnOffCompletes() {
        robot.restoreWalletOneAndOpenDetail(listedAfterRestore = EXTRA_COPY_RECORD_IDS)
        assertEquals("the copy of wallet one should be an extra copy", EXTRA_COPY_RECORD_IDS.toSet(), robot.extraCopyRecordIds())
        assertTrue("nothing should be only in the cloud", robot.manager.notOnDeviceRows.isEmpty())
        robot.manager.assertNextActions("extra copies alone should still offer Turn Off", available = setOf(TURN_OFF))

        robot.tapTurnOff()
        robot.waitForFinalDialog()
        assertTrue("the review should name the extra copy", device.hasObject(By.textContains("1 extra copy")))
        robot.confirmOnce()

        robot.waitForTurnedOff()
        assertEquals("Turn Off should delete the backup once", 1, ScriptedCloudStorageAccess.namespaceDeleteCount())
    }

    @Test
    fun walletsNotOnDeviceAskTwiceAndRestoreThemFirstRestoresThem() {
        robot.restoreWalletOneAndOpenDetail()

        robot.tapTurnOff()
        robot.waitForFirstDialog()
        assertTrue("the review should name the wallets only in the cloud", device.hasObject(By.textContains("Fixture Wallet Two")))
        assertFalse("a wallet only in the cloud asked once", device.hasObject(By.text(CONFIRM)))

        // the second dialog's Cancel leaves Turn Off as it was
        robot.clickText(CONTINUE)
        robot.waitForFinalDialog()
        robot.clickText(CANCEL)
        robot.assertTurnOffIdle(after = "cancelling the second dialog", deletesBefore = 0)

        // the first dialog's Cancel too
        robot.tapTurnOff()
        robot.waitForFirstDialog()
        robot.clickText(CANCEL)
        robot.assertTurnOffIdle(after = "cancelling the first dialog", deletesBefore = 0)

        robot.tapTurnOff()
        robot.waitForFirstDialog()
        robot.clickText(RESTORE_FIRST)
        robot.waitUntil("Restore Them First should restore the wallets only in the cloud", timeoutMs = 60_000L) {
            robot.manager.notOnDeviceRows.isEmpty() && AppManager.getInstance().wallets.size == 3
        }
        assertEquals("Restore Them First deleted a backup", 0, ScriptedCloudStorageAccess.namespaceDeleteCount())

        robot.manager.assertNextActions("once every wallet is on this device, Turn Off is offered", available = setOf(TURN_OFF))
        robot.tapTurnOff()
        robot.confirmOnce()
        robot.waitForTurnedOff()
    }

    @Test
    fun offlineTurnOffAsksForAConnectionThenCompletesAfterReconnect() {
        robot.freshEnableAndOpenDetail()
        val namespaces = ScriptedCloudStorageAccess.freshEnableNamespaces()

        ScriptedConnectivityAccess.setConnected(false)
        robot.tapTurnOff()
        robot.waitForText("No Connection")
        robot.waitUntil("Turn Off offline should wait for a connection") {
            robot.manager.configured?.disable == CloudBackupDisableScreen.Offline
        }
        assertEquals("Turn Off offline deleted a backup", 0, ScriptedCloudStorageAccess.namespaceDeleteCount())
        assertEquals(namespaces, ScriptedCloudStorageAccess.freshEnableNamespaces())

        ScriptedConnectivityAccess.setConnected(true)
        robot.manager.assertNextActions("reconnecting should offer Turn Off again", available = setOf(TURN_OFF))
        robot.clickText(TRY_AGAIN)

        robot.confirmOnce()
        robot.waitForTurnedOff()
        assertEquals("Turn Off left a backup in the cloud", emptyList<String>(), ScriptedCloudStorageAccess.freshEnableNamespaces())
    }

    @Test
    fun impactThatChangesBeforeConfirmBringsTheReviewBack() {
        robot.restoreWalletOneAndOpenDetail(listedAfterRestore = emptyList())
        robot.manager.assertNextActions(
            "with every wallet on this device, nothing is left to restore",
            available = setOf(TURN_OFF),
            unavailable = setOf(CloudBackupScreenAction.START_RESTORE_ALL),
        )

        robot.tapTurnOff()
        robot.waitForFinalDialog()
        assertFalse("a backup of this device's wallets asked twice", device.hasObject(By.text(FIRST_DIALOG_TITLE)))

        // wallets two and three appear in the cloud while the review is up, so confirming finds a
        // bigger impact than the one the user saw
        ScriptedCloudStorageAccess.exposeProductionFixtureWallets(
            listOf(ScriptedCloudBackupFixture.WALLET_RECORD_ID) + NOT_ON_DEVICE_RECORD_IDS,
        )
        robot.clickText(CONFIRM)

        robot.waitForFirstDialog()
        // the recheck lists the cloud without reading the new records, so it counts the two wallets
        // only in the cloud as unchecked rather than naming them
        val review = robot.manager.configured?.disable as? CloudBackupDisableScreen.Reviewing
        assertEquals(
            "the new review should count the wallets only in the cloud as unchecked; review=$review",
            NOT_ON_DEVICE_RECORD_IDS.size.toUInt(),
            review?.v1?.impact?.unchecked,
        )
        assertTrue(
            "the new review should say it can't check them",
            device.hasObject(By.textContains("Cove can't check ${NOT_ON_DEVICE_RECORD_IDS.size} backups")),
        )
        assertEquals("confirming an outdated review deleted a backup", 0, ScriptedCloudStorageAccess.namespaceDeleteCount())
        assertTrue(
            "the review should be back for the new impact",
            robot.manager.configured?.disable is CloudBackupDisableScreen.Reviewing,
        )

        robot.clickText(CANCEL)
        robot.assertTurnOffIdle(after = "cancelling the new review", deletesBefore = 0)
    }

    // Cancel, a tap outside the dialog and system Back
    @Test
    fun everyWayOutOfTheReviewLeavesTurnOffOffered() {
        robot.freshEnableAndOpenDetail()

        val waysOut =
            listOf<Pair<String, () -> Unit>>(
                "Cancel" to { robot.clickText(CANCEL) },
                "a tap outside the dialog" to { device.click(device.displayWidth / 2, device.displayHeight / 12) },
                "system Back" to { device.pressBack() },
            )

        for ((wayOut, leave) in waysOut) {
            robot.tapTurnOff()
            robot.waitForFinalDialog()
            robot.manager.assertNextActions(
                "the review should let the user confirm or cancel",
                available = setOf(CloudBackupScreenAction.CONFIRM_DISABLE, CloudBackupScreenAction.CANCEL_DISABLE),
            )
            leave()
            robot.assertTurnOffIdle(after = "leaving the review with $wayOut", deletesBefore = 0)
        }

        assertEquals("leaving the review changed the cloud", 1, ScriptedCloudStorageAccess.freshEnableNamespaces().size)
    }

    // the variant that fails and then taps Try Again in the same process; the relaunch is in
    // CloudBackupTurnOffRelaunchFullLaunchTest
    @Test
    fun failedDeleteThenTryAgainFinishesTurnOffInTheSameProcess() {
        robot.freshEnableAndOpenDetail()
        ScriptedCloudStorageAccess.failNamespaceDelete(attempt = 1)

        robot.tapTurnOff()
        robot.confirmOnce()
        // Stop Using is offered only after a failed Try Again, and nobody has tapped one yet
        robot.manager.assertNextActions(
            "a failed delete should leave Turn Off unfinished",
            body = CloudBackupScreenBody.FinishingDisable::class,
            available = setOf(CloudBackupScreenAction.RESUME_DISABLE),
            unavailable = setOf(TURN_OFF, CloudBackupScreenAction.STOP_USING_ON_THIS_DEVICE),
            timeoutMs = 60_000L,
        )
        robot.waitUntil("Settings should say Turn Off didn't finish") {
            robot.manager.screen.settingsRow == CloudBackupSettingsRowStatus.TurnOffIncomplete
        }

        // finish it from Settings, as a user would, so a finished Turn Off goes back there
        robot.openSettings()
        robot.clickText(TURN_OFF_INCOMPLETE_ROW)
        assertTrue(
            "the finishing panel isn't drawn",
            device.wait(Until.hasObject(By.res(CloudBackupTags.FINISHING_DISABLE)), 10_000L),
        )

        robot.clickTagged(CloudBackupTags.action(CloudBackupScreenAction.RESUME_DISABLE), scroll = true)
        robot.waitForTurnedOff()
        assertEquals("Try Again left a backup in the cloud", emptyList<String>(), ScriptedCloudStorageAccess.freshEnableNamespaces())
        robot.assertBackOnSettingsWithEnable()
    }

    // in one process; the relaunch is in CloudBackupTurnOffRelaunchFullLaunchTest
    @Test
    fun passkeyNotCompletedKeepsWalletsAndTurnOff() {
        robot.restoreWalletOneAndOpenDetail()
        val wallets = AppManager.getInstance().wallets.map { it.id }
        ScriptedPasskeyProvider.configureResults(Invocation.AUTHENTICATE, Result.NOT_COMPLETED)
        ScriptedPasskeyProvider.configureResults(Invocation.DISCOVER, Result.NOT_COMPLETED)

        robot.clickTagged(CloudBackupTags.action(CloudBackupScreenAction.VERIFY), scroll = true)
        robot.waitUntil("a dismissed passkey prompt should leave verification not completed") {
            robot.manager.verificationState == CloudBackupVerificationState.NotCompleted
        }
        robot.manager.assertNextActions(
            "a dismissed passkey prompt should keep Turn Off",
            available = setOf(TURN_OFF, CloudBackupScreenAction.VERIFY),
        )
        // the card shows above Verify, which the tap scrolled to, so it can sit above the screen
        robot.scrollToText("Cloud Backup Not Verified")
        robot.scrollToText("Verification didn't finish. Verify again to confirm your cloud backup can be restored.")
        assertFalse("a dismissed passkey prompt offered a passkey repair", device.hasObject(By.text("Add New Passkey")))
        assertFalse(
            "a dismissed passkey prompt said the passkey may be deleted",
            device.hasObject(By.textContains("If your passkey was deleted")),
        )
        robot.manager.assertNoRootPrompt("a dismissed passkey prompt raised a prompt of its own")
        assertEquals("the wallets changed", wallets, AppManager.getInstance().wallets.map { it.id })
    }

    @Test
    fun heldRowDeleteHoldsTurnOffUntilItFinishes() {
        robot.restoreWalletOneAndOpenDetail()
        val recordId = NOT_ON_DEVICE_RECORD_IDS.first()
        ScriptedCloudStorageAccess.holdNextWalletDelete()

        // the row's own confirmation is covered by CloudBackupDeleteFromCloudFullLaunchTest, so the
        // delete is dispatched as its confirm button sends it
        InstrumentationRegistry.getInstrumentation().runOnMainSync {
            robot.manager.dispatch(CloudBackupManagerAction.DeleteCloudWallet(recordId))
        }
        assertTrue("the row delete never reached the provider", ScriptedCloudStorageAccess.awaitHeldWalletDelete())

        robot.manager.assertNextActions("a running row delete should hold Turn Off", unavailable = setOf(TURN_OFF))
        val turnOff = robot.manager.availability(TURN_OFF)
        assertTrue("Turn Off is hidden, not unavailable: $turnOff", turnOff is CloudBackupActionAvailability.Unavailable)
        val caption = requireNotNull(turnOff?.caption) { "unavailable Turn Off carries no reason" }
        robot.waitUntil("unavailable Turn Off shows no reason") {
            device.findObject(By.text(caption)) != null || scrolledTo(caption)
        }

        ScriptedCloudStorageAccess.releaseHeldWalletDelete()
        robot.manager.assertNextActions(
            "Turn Off should come back once the delete finishes",
            available = setOf(TURN_OFF),
            timeoutMs = 60_000L,
        )
        assertEquals("the held delete didn't finish", listOf(recordId), ScriptedCloudStorageAccess.deletedWalletRecordIds())
    }

    @Test
    fun lockDuringTheReviewKeepsTheScreenCoherent() {
        robot.freshEnableAndOpenDetail()
        setMainPin()

        robot.tapTurnOff()
        robot.waitForFinalDialog()

        InstrumentationRegistry.getInstrumentation().runOnMainSync {
            AuthManager.getInstance().lock(AppLockReason.REAUTHENTICATION)
        }
        robot.waitForText(LOCK_TITLE)
        assertFalse("the review showed over the lock", device.hasObject(By.text(CONFIRM)))
        assertTrue(
            "locking left the review",
            robot.manager.configured?.disable is CloudBackupDisableScreen.Reviewing,
        )

        enterPin()
        robot.waitUntil("the PIN should unlock the app") { !AuthManager.getInstance().isLocked }
        robot.waitForFinalDialog()
        robot.manager.assertNextActions(
            "unlocking should bring back the same review",
            available = setOf(CloudBackupScreenAction.CONFIRM_DISABLE, CloudBackupScreenAction.CANCEL_DISABLE),
        )
        assertEquals("locking deleted a backup", 0, ScriptedCloudStorageAccess.namespaceDeleteCount())

        robot.clickText(CANCEL)
        robot.assertTurnOffIdle(after = "cancelling the review after unlocking", deletesBefore = 0)
    }

    private fun setMainPin() {
        val auth = AuthManager.getInstance()
        InstrumentationRegistry.getInstrumentation().runOnMainSync { auth.dispatch(AuthManagerAction.SetPin(MAIN_PIN)) }
        robot.waitUntil("the main PIN should enable the app lock") { auth.isAuthEnabled }
    }

    // the lock cover slides up, so a key found mid-slide would take the tap meant for its neighbour
    private fun enterPin() {
        device.wait(Until.findObject(By.res("numberPadPin.digit.0")), 10_000L) ?: error("the number pad never showed")
        Thread.sleep(PIN_PAD_SETTLE_MS)

        MAIN_PIN.forEach { digit ->
            val key =
                device.wait(Until.findObject(By.res("numberPadPin.digit.$digit")), 5_000L)
                    ?: error("number pad key $digit is missing")
            key.click()
        }
    }

    private fun scrolledTo(text: String): Boolean {
        device.swipe(device.displayWidth / 2, device.displayHeight * 3 / 4, device.displayWidth / 2, device.displayHeight / 4, 20)
        device.waitForIdle()
        return device.findObject(By.text(text)) != null
    }

    private companion object {
        const val LOCK_TITLE = "Enter Cove PIN"
        const val MAIN_PIN = "135791"
        const val PIN_PAD_SETTLE_MS = 1_000L
    }
}
