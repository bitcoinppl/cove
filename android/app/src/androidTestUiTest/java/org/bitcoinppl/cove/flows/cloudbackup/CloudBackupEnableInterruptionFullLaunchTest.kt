package org.bitcoinppl.cove.flows.cloudbackup

import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.filters.LargeTest
import androidx.test.uiautomator.By
import androidx.test.uiautomator.UiDevice
import org.bitcoinppl.cove.cloudbackup.CloudBackupManager
import org.bitcoinppl.cove.test.FullLaunchOnboardingRobot
import org.bitcoinppl.cove.test.FullLaunchStartupRobot
import org.bitcoinppl.cove.test.FullLaunchTestRule
import org.bitcoinppl.cove.test.ManualFullLaunchTest
import org.bitcoinppl.cove.test.StagedProcessFullLaunchTest
import org.bitcoinppl.cove.test.fullLaunchDevice
import org.bitcoinppl.cove.test.launchFullApp
import org.bitcoinppl.cove.testconfig.HoldableKeychainAccess
import org.bitcoinppl.cove.testconfig.ScriptedCloudStorageAccess
import org.bitcoinppl.cove.testconfig.ScriptedCloudStorageAccess.UploadKind
import org.bitcoinppl.cove.testconfig.ScriptedPasskeyProvider
import org.bitcoinppl.cove.testconfig.ScriptedPasskeyProvider.Invocation
import org.bitcoinppl.cove_core.CloudBackupVerificationState
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

/**
 * Kills a fresh Cloud Backup enable at passkey registration, passkey confirmation and the end of
 * local promotion, then relaunches it in a new process
 *
 * Each scenario's stages run in order in separate processes through
 * `cargo xtask android-ui-enable-interruption`: stage one holds the call and ends with the app
 * still inside it, and the runner force-stops the process before stage two relaunches the app
 * against the same cloud and keychain
 */
@ManualFullLaunchTest
@LargeTest
@RunWith(AndroidJUnit4::class)
class CloudBackupEnableInterruptionFullLaunchTest {
    private lateinit var device: UiDevice

    @get:Rule
    val fullLaunch = FullLaunchTestRule()

    @Before
    fun prepareDevice() {
        ScriptedPasskeyProvider.reset()
        device = fullLaunchDevice()
    }

    @Test
    @StagedProcessFullLaunchTest
    fun registrationStage1KillWhileThePasskeyIsRegistered() {
        startFirstProcess()
        ScriptedPasskeyProvider.creationHold.holdNext()
        launchFullApp()

        startFreshCloudBackupEnable()
        assertTrue("the enable never asked to register a passkey", ScriptedPasskeyProvider.creationHold.awaitEntered())
        assertEquals("the enable wrote to the cloud before registering", 0, ScriptedCloudStorageAccess.uploadCount(UploadKind.MASTER))
        assertTrue("the enable never claimed its staged key", HoldableKeychainAccess.hasPendingEnableJournal())
    }

    // nothing was registered, so the relaunch drops the staged key and offers a fresh enable
    // instead of a passkey to confirm
    @Test
    @StagedProcessFullLaunchTest
    fun registrationStage2RelaunchStartsOver() {
        relaunchSecondProcess()

        // the lifecycle reads disabled before restart recovery runs, so the journal's removal is
        // what shows the staged key was discarded
        waitUntil("relaunch should discard the interrupted enable") {
            !HoldableKeychainAccess.hasPendingEnableJournal() &&
                CloudBackupManager.getInstance().isDisabled
        }
        assertEquals("the interrupted enable wrote to the cloud", emptyList<String>(), ScriptedCloudStorageAccess.freshEnableNamespaces())

        // with no passkey to confirm, onboarding resumes either on the Cloud Backup details or on
        // its choice of backup method; iOS runs have landed on both
        val onboarding = FullLaunchOnboardingRobot(device)
        if (awaitDetailsOrBackupStep()) onboarding.openCloudBackupFromBackupWallet()
        onboarding.assertCloudBackupDetails()
        assertNull("relaunch offered a passkey that was never registered", device.findObject(By.text("Confirm Passkey")))

        onboarding.enableCloudBackupFromDetails()
        assertTrue(
            "expected the fresh enable to write its master and wallet backups",
            ScriptedCloudStorageAccess.awaitAllProviderWritesAccepted(),
        )
        onboarding.assertCloudBackupSuccess()
        assertEquals("the fresh enable should register its own passkey", 1, ScriptedPasskeyProvider.callCount(Invocation.CREATE))
        assertEquals("the cloud holds more than one backup", 1, ScriptedCloudStorageAccess.freshEnableNamespaces().size)
    }

    @Test
    @StagedProcessFullLaunchTest
    fun confirmationStage1KillWhileTheNewPasskeyIsConfirmed() {
        startFirstProcess()
        ScriptedPasskeyProvider.authenticationHold.holdNext()
        launchFullApp()

        startFreshCloudBackupEnable()
        assertTrue(
            "the enable never confirmed its new passkey",
            ScriptedPasskeyProvider.authenticationHold.awaitEntered(),
        )
        assertEquals(1, ScriptedPasskeyProvider.callCount(Invocation.CREATE))
        assertEquals(
            "the enable uploaded before the passkey was confirmed",
            0,
            ScriptedCloudStorageAccess.uploadCount(UploadKind.MASTER),
        )
    }

    // the passkey was registered, so the relaunch offers to confirm it and never registers another
    @Test
    @StagedProcessFullLaunchTest
    fun confirmationStage2RelaunchConfirmsTheSamePasskey() {
        relaunchSecondProcess()

        val onboarding =
            FullLaunchOnboardingRobot(device)
                .assertPendingEnableRecovery()
                .enableCloudBackupFromDetails()
        assertTrue(
            "expected the confirmed enable to write its master and wallet backups",
            ScriptedCloudStorageAccess.awaitAllProviderWritesAccepted(),
        )
        onboarding.assertCloudBackupSuccess()

        assertEquals("recovery registered another passkey", 0, ScriptedPasskeyProvider.callCount(Invocation.CREATE))
        assertEquals(
            "recovery confirmed another passkey",
            setOf(ScriptedPasskeyProvider.knownCredentialIdHex()),
            ScriptedPasskeyProvider.authenticatedCredentialIds().toSet(),
        )
        assertEquals("the cloud holds more than one backup", 1, ScriptedCloudStorageAccess.freshEnableNamespaces().size)
    }

    @Test
    @StagedProcessFullLaunchTest
    fun promotionStage1KillAsPromotionEnds() {
        startFirstProcess()
        HoldableKeychainAccess.pendingEnableJournalDelete.holdNext()
        launchFullApp()

        startFreshCloudBackupEnable()
        assertTrue(
            "the enable never reached the end of its local promotion",
            HoldableKeychainAccess.pendingEnableJournalDelete.awaitEntered(),
        )
        assertTrue("promotion started before the cloud accepted every write", ScriptedCloudStorageAccess.awaitAllProviderWritesAccepted())
        assertEquals(1, ScriptedPasskeyProvider.callCount(Invocation.CREATE))
        assertEquals(1, ScriptedCloudStorageAccess.freshEnableNamespaces().size)

        // the writes are readable before the relaunch, so verification has nothing to repair and
        // any upload after it is the relaunch writing the backup again
        ScriptedCloudStorageAccess.releaseVisibility()
    }

    // the promotion committed the new key and passkey before the kill, so the relaunch finishes
    // the enable on its own, with no prompt and nothing uploaded again
    @Test
    @StagedProcessFullLaunchTest
    fun promotionStage2RelaunchFinishesTheEnable() {
        relaunchSecondProcess()

        waitUntil("relaunch should finish the promoted enable") {
            CloudBackupManager.getInstance().isConfigured
        }
        assertNull(
            "relaunch asked to confirm a passkey the promotion already committed",
            device.findObject(By.text("Confirm Passkey")),
        )

        // the finished backup then confirms in the background like any other
        CloudBackupManager.getInstance().resumePendingCloudUploadVerification()
        waitUntil("the promoted backup never verified", timeoutMs = 60_000) {
            val manager = CloudBackupManager.getInstance()
            !manager.hasPendingUploadVerification &&
                manager.verificationState is CloudBackupVerificationState.Verified
        }

        assertEquals("the relaunch registered another passkey", 0, ScriptedPasskeyProvider.callCount(Invocation.CREATE))
        assertEquals("the relaunch asked for the passkey", 0, passkeyAssertionCount())
        assertEquals("the relaunch uploaded the master key again", 0, ScriptedCloudStorageAccess.uploadCount(UploadKind.MASTER))
        assertEquals("the relaunch uploaded a wallet again", 0, ScriptedCloudStorageAccess.uploadCount(UploadKind.WALLET))
        assertEquals("the relaunch wrote another backup", 1, ScriptedCloudStorageAccess.freshEnableNamespaces().size)
    }

    private fun startFirstProcess() {
        ScriptedCloudStorageAccess.configurePersistentFreshEnableWithDelayedVisibility(resetStoredState = true)
        assertTrue(
            "expected the first process of the scenario",
            ScriptedCloudStorageAccess.recordProcessAndCheckRestart(expectedRestart = false),
        )
    }

    private fun relaunchSecondProcess() {
        ScriptedCloudStorageAccess.configurePersistentFreshEnableWithDelayedVisibility(resetStoredState = false)
        assertTrue(
            "expected stage two to run in a fresh application process",
            ScriptedCloudStorageAccess.recordProcessAndCheckRestart(expectedRestart = true),
        )
        launchFullApp(resetData = false)
    }

    private fun startFreshCloudBackupEnable(): FullLaunchOnboardingRobot {
        FullLaunchStartupRobot(device).assertBootstrappedIntoOnboarding()

        return FullLaunchOnboardingRobot(device)
            .tapGetStarted()
            .chooseNewUser()
            .openCloudBackupFromBackupWallet()
            .assertCloudBackupDetails()
            .enableCloudBackupFromDetails()
    }

    private fun passkeyAssertionCount(): Int =
        ScriptedPasskeyProvider.callCount(Invocation.AUTHENTICATE) +
            ScriptedPasskeyProvider.callCount(Invocation.DISCOVER)

    /** Waits for the Cloud Backup details or the backup step and returns true on the backup step */
    private fun awaitDetailsOrBackupStep(): Boolean {
        val deadline = System.currentTimeMillis() + 30_000

        while (System.currentTimeMillis() < deadline) {
            if (device.hasObject(By.text("How It Works"))) return false
            if (device.hasObject(By.textContains("Back up your wallet"))) return true

            Thread.sleep(200)
        }

        throw AssertionError("relaunch showed neither the Cloud Backup details nor the backup step")
    }

    private fun waitUntil(
        message: String,
        timeoutMs: Long = 30_000,
        condition: () -> Boolean,
    ) {
        val deadline = System.currentTimeMillis() + timeoutMs

        while (System.currentTimeMillis() < deadline) {
            if (condition()) return

            Thread.sleep(100)
        }

        val manager = CloudBackupManager.getInstance()
        assertTrue("$message; ${manager.describeScreen()}", condition())
    }
}
