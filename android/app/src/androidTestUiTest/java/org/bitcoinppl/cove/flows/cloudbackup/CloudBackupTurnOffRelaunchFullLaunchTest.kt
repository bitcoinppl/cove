package org.bitcoinppl.cove.flows.cloudbackup

import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.filters.LargeTest
import androidx.test.uiautomator.By
import androidx.test.uiautomator.UiDevice
import androidx.test.uiautomator.Until
import org.bitcoinppl.cove.cloudbackup.CloudBackupTags
import org.bitcoinppl.cove.flows.cloudbackup.CloudBackupTurnOffRobot.Companion.LOCAL_MASTER_KEY_ENTRIES
import org.bitcoinppl.cove.flows.cloudbackup.CloudBackupTurnOffRobot.Companion.NOT_ON_DEVICE_RECORD_IDS
import org.bitcoinppl.cove.flows.cloudbackup.CloudBackupTurnOffRobot.Companion.TURN_OFF
import org.bitcoinppl.cove.flows.cloudbackup.CloudBackupTurnOffRobot.Companion.TURN_OFF_INCOMPLETE_ROW
import org.bitcoinppl.cove.flows.cloudbackup.CloudBackupTurnOffRobot.Companion.VERIFY_FIRST
import org.bitcoinppl.cove.test.FullLaunchTestRule
import org.bitcoinppl.cove.test.ManualFullLaunchTest
import org.bitcoinppl.cove.test.StagedProcessFullLaunchTest
import org.bitcoinppl.cove.test.fullLaunchDevice
import org.bitcoinppl.cove.testconfig.HoldableKeychainAccess
import org.bitcoinppl.cove.testconfig.LegacyStuckRecordSeed
import org.bitcoinppl.cove.testconfig.ScriptedCloudBackupFixture
import org.bitcoinppl.cove.testconfig.ScriptedCloudStorageAccess
import org.bitcoinppl.cove.testconfig.ScriptedPasskeyProvider
import org.bitcoinppl.cove.testconfig.ScriptedPasskeyProvider.Invocation
import org.bitcoinppl.cove.testconfig.ScriptedPasskeyProvider.Result
import org.bitcoinppl.cove_core.CloudBackupScreenAction
import org.bitcoinppl.cove_core.CloudBackupScreenBody
import org.bitcoinppl.cove_core.CloudBackupSettingsRowStatus
import org.bitcoinppl.cove_core.CloudBackupVerificationState
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

/**
 * The Turn Off scenarios that span a relaunch, each a pair of stages `xtask android-ui-turn-off`
 * runs in order, force-stopping the app between them
 *
 * The scripted passkey provider counts calls per process, so a second stage that sees no calls
 * proves the relaunch asked for no passkey
 */
@ManualFullLaunchTest
@LargeTest
@RunWith(AndroidJUnit4::class)
class CloudBackupTurnOffRelaunchFullLaunchTest {
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

    // the wallets only in the cloud can't be checked without the local master key
    @Test
    @StagedProcessFullLaunchTest
    fun uncheckedStage1RestoreThenDropTheLocalMasterKey() {
        robot.restoreWalletOneAndOpenDetail()
        recordTheFirstFixtureProcess()
        robot.manager.assertNextActions(
            "with the local key, the wallets only in the cloud can be restored",
            available = setOf(TURN_OFF, CloudBackupScreenAction.START_RESTORE_ALL),
        )

        // Cove caches the key in memory, so the relaunch is what makes it read the missing entries
        LOCAL_MASTER_KEY_ENTRIES.forEach(HoldableKeychainAccess::delete)
        LOCAL_MASTER_KEY_ENTRIES.forEach { key -> assertNull("$key is still in the keychain", HoldableKeychainAccess.get(key)) }
    }

    @Test
    @StagedProcessFullLaunchTest
    fun uncheckedStage2VerifyFirstFailsNeutrallyAndTurnOffStillAsksTwice() {
        // the passkey is dismissed from the start, or the relaunch's first check would recover the
        // key through it and check every row
        ScriptedPasskeyProvider.configureResults(Invocation.AUTHENTICATE, Result.NOT_COMPLETED)
        ScriptedPasskeyProvider.configureResults(Invocation.DISCOVER, Result.NOT_COMPLETED)
        relaunchOnTheFixture()
        robot.openCloudBackup()
        robot.waitUntil("without the local key the wallets only in the cloud should be unchecked", timeoutMs = 60_000L) {
            robot.uncheckedRowCount() == NOT_ON_DEVICE_RECORD_IDS.size
        }

        robot.tapTurnOff()
        robot.waitForFirstDialog()
        assertTrue("the review should say why it can't check them", device.hasObject(By.textContains("without your passkey")))
        robot.clickText(VERIFY_FIRST)

        robot.waitUntil("a dismissed passkey prompt should leave verification not completed") {
            robot.manager.verificationState == CloudBackupVerificationState.NotCompleted
        }
        robot.waitForText("Cloud Backup Not Verified")
        robot.manager.assertNextActions("Verify First failing should keep Turn Off", available = setOf(TURN_OFF))
        assertEquals("Verify First deleted a backup", 0, ScriptedCloudStorageAccess.namespaceDeleteCount())

        robot.tapTurnOff()
        robot.confirmTwice()
        robot.waitForTurnedOff()
    }

    @Test
    @StagedProcessFullLaunchTest
    fun failedDeleteStage1ConfirmTurnOffWhoseDeleteFails() {
        robot.freshEnableAndOpenDetail(persistent = true)
        // a failed delete retries on its own once the cloud is reachable, so every delete fails
        // until the process ends
        ScriptedCloudStorageAccess.failNamespaceDeletes(1..50)

        robot.tapTurnOff()
        robot.confirmOnce()
        robot.manager.assertNextActions(
            "a failed delete should leave Turn Off unfinished",
            body = CloudBackupScreenBody.FinishingDisable::class,
            available = setOf(CloudBackupScreenAction.RESUME_DISABLE),
            timeoutMs = 60_000L,
        )
    }

    @Test
    @StagedProcessFullLaunchTest
    fun failedDeleteStage2RelaunchShowsTurnOffIncompleteAndTryAgainFinishes() {
        ScriptedCloudStorageAccess.configurePersistentFreshEnableWithDelayedVisibility(resetStoredState = false)
        assertTrue(
            "expected the second stage to run in a fresh application process",
            ScriptedCloudStorageAccess.recordProcessAndCheckRestart(expectedRestart = true),
        )

        // a launch resumes a committed Turn Off at once, and scripted failures don't outlive a
        // process, so this one fails every delete until the test taps Try Again
        ScriptedCloudStorageAccess.failNamespaceDeletes(1..50)
        robot.launchApp(resetData = false)
        robot.waitUntil("the relaunch should resume the committed Turn Off") {
            ScriptedCloudStorageAccess.namespaceDeleteCount() >= 1
        }

        robot.openSettings()
        robot.waitUntil("Settings should say Turn Off didn't finish") {
            robot.manager.screen.settingsRow == CloudBackupSettingsRowStatus.TurnOffIncomplete
        }
        robot.waitForText(TURN_OFF_INCOMPLETE_ROW)
        // Stop Using is offered only after a failed Try Again, and nobody has tapped one yet
        robot.manager.assertNextActions(
            "the relaunch should still be finishing Turn Off",
            body = CloudBackupScreenBody.FinishingDisable::class,
            available = setOf(CloudBackupScreenAction.RESUME_DISABLE),
            unavailable = setOf(TURN_OFF, CloudBackupScreenAction.STOP_USING_ON_THIS_DEVICE),
        )

        // the panel needs no inventory, which this process never loaded
        robot.clickText(TURN_OFF_INCOMPLETE_ROW)
        assertTrue(
            "the finishing panel isn't drawn",
            device.wait(Until.hasObject(By.res(CloudBackupTags.FINISHING_DISABLE)), 10_000L),
        )
        ScriptedCloudStorageAccess.stopFailingNamespaceDeletes()
        robot.clickTagged(CloudBackupTags.action(CloudBackupScreenAction.RESUME_DISABLE), scroll = true)

        robot.waitForTurnedOff()
        assertEquals("Try Again left a backup in the cloud", emptyList<String>(), ScriptedCloudStorageAccess.freshEnableNamespaces())
        robot.assertBackOnSettingsWithEnable()
    }

    @Test
    @StagedProcessFullLaunchTest
    fun passkeyNotCompletedStage1DismissThePasskeyPrompt() {
        robot.restoreWalletOneAndOpenDetail()
        recordTheFirstFixtureProcess()
        ScriptedPasskeyProvider.configureResults(Invocation.AUTHENTICATE, Result.NOT_COMPLETED)
        ScriptedPasskeyProvider.configureResults(Invocation.DISCOVER, Result.NOT_COMPLETED)

        robot.clickTagged(CloudBackupTags.action(CloudBackupScreenAction.VERIFY), scroll = true)
        robot.waitUntil("a dismissed passkey prompt should leave verification not completed") {
            robot.manager.verificationState == CloudBackupVerificationState.NotCompleted
        }
        robot.manager.assertNextActions("a dismissed passkey prompt should keep Turn Off", available = setOf(TURN_OFF))
    }

    @Test
    @StagedProcessFullLaunchTest
    fun passkeyNotCompletedStage2RelaunchKeepsTurnOffWithoutPrompting() {
        relaunchOnTheFixture()
        robot.openSettings()

        robot.manager.assertNextActions("the relaunch should keep Turn Off", available = setOf(TURN_OFF))
        Thread.sleep(RELAUNCH_SETTLE_MS)
        assertEquals("the relaunch asked for a passkey", listOf(0, 0, 0), robot.passkeyCalls())
        robot.manager.assertNoRootPrompt("the relaunch raised a prompt")
    }

    @Test
    @StagedProcessFullLaunchTest
    fun healStage1RequestTheLegacyStuckRecord() {
        robot.freshEnableAndOpenDetail(persistent = true)
        robot.manager.assertNextActions("the backup to heal should start out working", available = setOf(TURN_OFF))
        LegacyStuckRecordSeed.requestOnNextLaunch()
    }

    @Test
    @StagedProcessFullLaunchTest
    fun healStage2NoticeShowsWithoutAPasskeyAndTurnsOffFromIt() {
        ScriptedCloudStorageAccess.configurePersistentFreshEnableWithDelayedVisibility(resetStoredState = false)
        assertTrue(
            "expected the second stage to run in a fresh application process",
            ScriptedCloudStorageAccess.recordProcessAndCheckRestart(expectedRestart = true),
        )
        robot.launchApp(resetData = false)
        assertTrue("Rust never seeded the legacy stuck record", LegacyStuckRecordSeed.awaitSeeded())

        robot.openSettings()
        robot.waitUntil("the healed backup should say it is on again") {
            robot.manager.screen.settingsRow == CloudBackupSettingsRowStatus.TurnedOnAgain &&
                robot.manager.screen.healNotice != null
        }
        robot.waitForText(TURNED_ON_AGAIN_ROW)
        robot.manager.assertNextActions(
            "the healed backup should offer Turn Off",
            available = setOf(TURN_OFF, CloudBackupScreenAction.DISMISS_HEAL_NOTICE),
        )

        robot.clickText(TURNED_ON_AGAIN_ROW)
        assertTrue("the heal notice isn't drawn", device.wait(Until.hasObject(By.res(CloudBackupTags.HEAL_NOTICE)), 10_000L))
        assertEquals("healing asked for a passkey", listOf(0, 0, 0), robot.passkeyCalls())

        robot.clickTagged(CloudBackupTags.HEAL_NOTICE_TURN_OFF, scroll = true)
        robot.confirmOnce()
        robot.waitForTurnedOff()
        assertEquals("Turn Off left a backup in the cloud", emptyList<String>(), ScriptedCloudStorageAccess.freshEnableNamespaces())
    }

    /** Relaunches on the fixture cloud a first stage restored from, with wallets two and three listed */
    private fun relaunchOnTheFixture() {
        ScriptedCloudStorageAccess.configureProductionFixtureRestore()
        ScriptedCloudStorageAccess.exposeProductionFixtureWallets(
            listOf(ScriptedCloudBackupFixture.WALLET_RECORD_ID) + NOT_ON_DEVICE_RECORD_IDS,
        )
        assertTrue(
            "expected the second stage to run in a fresh application process",
            ScriptedCloudStorageAccess.recordFixtureProcessAndCheckRestart(resetStoredState = false, expectedRestart = true),
        )
        robot.launchApp(resetData = false)
    }

    private fun recordTheFirstFixtureProcess() {
        assertTrue(
            "expected the first process of a staged Turn Off scenario",
            ScriptedCloudStorageAccess.recordFixtureProcessAndCheckRestart(resetStoredState = true, expectedRestart = false),
        )
    }

    private companion object {
        const val TURNED_ON_AGAIN_ROW = "Turned on again — open to review"

        // long enough for a relaunch's background checks to have asked for a passkey if they would
        const val RELAUNCH_SETTLE_MS = 5_000L
    }
}
