package org.bitcoinppl.cove.flows.cloudbackup

import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.filters.LargeTest
import androidx.test.uiautomator.UiDevice
import org.bitcoinppl.cove.cloudbackup.CloudBackupManager
import org.bitcoinppl.cove.test.FullLaunchOnboardingRobot
import org.bitcoinppl.cove.test.FullLaunchStartupRobot
import org.bitcoinppl.cove.test.FullLaunchTestRule
import org.bitcoinppl.cove.test.ManualFullLaunchTest
import org.bitcoinppl.cove.test.StagedProcessFullLaunchTest
import org.bitcoinppl.cove.test.fullLaunchDevice
import org.bitcoinppl.cove.test.launchFullApp
import org.bitcoinppl.cove.test.recreateFullAppActivity
import org.bitcoinppl.cove.testconfig.ScriptedCloudStorageAccess
import org.bitcoinppl.cove.testconfig.ScriptedPasskeyProvider
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

@ManualFullLaunchTest
@LargeTest
@RunWith(AndroidJUnit4::class)
class CloudBackupDurableCompletionFullLaunchTest {
    private lateinit var device: UiDevice

    @get:Rule
    val fullLaunch = FullLaunchTestRule()

    @Before
    fun configureAndLaunchActivity() {
        ScriptedPasskeyProvider.reset()
        device = fullLaunchDevice()
    }

    @Test
    fun acceptedWritesAdvanceOnboardingBeforeVisibilityAndConfirmInBackground() {
        ScriptedCloudStorageAccess.configureFreshEnableWithDelayedVisibility()
        launchFullApp()

        val onboarding = startFreshCloudBackupEnable()
        assertTrue("expected test passkey creation", ScriptedPasskeyProvider.awaitCreation())
        assertTrue(
            "expected the cloud provider to accept the master and wallet writes",
            ScriptedCloudStorageAccess.awaitAllProviderWritesAccepted(),
        )

        onboarding.assertCloudBackupSuccess()
        assertFalse(
            "onboarding must advance before provider reads expose the accepted writes",
            ScriptedCloudStorageAccess.isVisibilityReleased(),
        )
        waitUntil("expected durable upload confirmation to remain pending") {
            CloudBackupManager.getInstance().hasPendingUploadVerification
        }

        ScriptedCloudStorageAccess.releaseVisibility()
        CloudBackupManager.getInstance().resumePendingCloudUploadVerification()

        assertTrue(
            "expected background confirmation to read the newly visible backups",
            ScriptedCloudStorageAccess.awaitVisibleConfirmationRead(),
        )
        waitUntil("expected background confirmation to clear the pending state") {
            !CloudBackupManager.getInstance().hasPendingUploadVerification
        }
    }

    @Test
    fun activityRecreationFailsClosedDuringPendingEnableAndLaterCompletes() {
        ScriptedCloudStorageAccess.configureFreshEnableWithDelayedVisibility(
            blockMasterUploadReturn = true,
        )
        launchFullApp()

        val onboarding = startFreshCloudBackupEnable()
        assertTrue("expected test passkey creation", ScriptedPasskeyProvider.awaitCreation())
        assertTrue(
            "expected the cloud provider to accept the master write",
            ScriptedCloudStorageAccess.awaitMasterWriteAccepted(),
        )

        try {
            recreateFullAppActivity()
            onboarding.assertCloudBackupEnableRemainsPending()

            ScriptedCloudStorageAccess.releaseMasterUploadReturn()
            assertTrue(
                "expected the cloud provider to accept the master and wallet writes",
                ScriptedCloudStorageAccess.awaitAllProviderWritesAccepted(),
            )
            onboarding.assertCloudBackupSuccess()

            ScriptedCloudStorageAccess.releaseVisibility()
            CloudBackupManager.getInstance().resumePendingCloudUploadVerification()
            assertTrue(
                "expected confirmation after activity recreation",
                ScriptedCloudStorageAccess.awaitVisibleConfirmationRead(),
            )
            waitUntil("expected confirmation after activity recreation to clear pending state") {
                !CloudBackupManager.getInstance().hasPendingUploadVerification
            }
        } finally {
            ScriptedCloudStorageAccess.releaseMasterUploadReturn()
            ScriptedCloudStorageAccess.releaseVisibility()
        }
    }

    @Test
    @StagedProcessFullLaunchTest
    fun processStage1InterruptAfterProviderAcceptsWrite() {
        ScriptedCloudStorageAccess.configurePersistentFreshEnableWithDelayedVisibility(
            resetStoredState = true,
            blockMasterUploadReturn = true,
        )
        assertTrue(
            "expected the first process in the durable relaunch scenario",
            ScriptedCloudStorageAccess.recordProcessAndCheckRestart(expectedRestart = false),
        )
        launchFullApp()

        startFreshCloudBackupEnable()
        assertTrue("expected test passkey creation", ScriptedPasskeyProvider.awaitCreation())
        assertTrue(
            "expected a provider write after the pending-enable journal became durable",
            ScriptedCloudStorageAccess.awaitMasterWriteAccepted(),
        )
    }

    @Test
    @StagedProcessFullLaunchTest
    fun processStage2RelaunchFailsClosedThenCompletesAcceptedWrites() {
        ScriptedCloudStorageAccess.configurePersistentFreshEnableWithDelayedVisibility(
            resetStoredState = false,
        )
        assertTrue(
            "expected stage two to run in a fresh application process",
            ScriptedCloudStorageAccess.recordProcessAndCheckRestart(expectedRestart = true),
        )
        launchFullApp(resetData = false)

        val onboarding =
            FullLaunchOnboardingRobot(device)
                .assertPendingEnableRecovery()
                .enableCloudBackupFromDetails()
        assertTrue(
            "expected the relaunched upload to restore both persisted provider writes",
            ScriptedCloudStorageAccess.awaitAllProviderWritesAccepted(),
        )

        onboarding.assertCloudBackupSuccess()
        assertFalse(
            "relaunch completion must not require immediate provider read visibility",
            ScriptedCloudStorageAccess.isVisibilityReleased(),
        )
        waitUntil("expected durable upload confirmation after relaunch") {
            CloudBackupManager.getInstance().hasPendingUploadVerification
        }
    }

    @Test
    @StagedProcessFullLaunchTest
    fun processStage3RelaunchConfirmsPersistedWritesInBackground() {
        ScriptedCloudStorageAccess.configurePersistentFreshEnableWithDelayedVisibility(
            resetStoredState = false,
        )
        assertTrue(
            "expected stage three to run in a fresh application process",
            ScriptedCloudStorageAccess.recordProcessAndCheckRestart(expectedRestart = true),
        )
        ScriptedCloudStorageAccess.releaseVisibility()
        launchFullApp(resetData = false)
        CloudBackupManager.getInstance().resumePendingCloudUploadVerification()

        assertTrue(
            "expected a fresh process to confirm the persisted provider writes",
            ScriptedCloudStorageAccess.awaitVisibleConfirmationRead(),
        )
        waitUntil("expected fresh-process confirmation to clear pending state") {
            !CloudBackupManager.getInstance().hasPendingUploadVerification
        }
        FullLaunchOnboardingRobot(device).assertCloudBackupSuccess()
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

    private fun waitUntil(
        message: String,
        timeoutMs: Long = 20_000,
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
