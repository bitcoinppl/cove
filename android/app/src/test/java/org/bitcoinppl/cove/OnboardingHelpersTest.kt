package org.bitcoinppl.cove

import org.bitcoinppl.cove.flows.OnboardingFlow.combinedRestoreProgress
import org.bitcoinppl.cove.flows.OnboardingFlow.isOnboardingCloudBackupEnableCompletion
import org.bitcoinppl.cove.flows.OnboardingFlow.shouldCompleteOnboardingCloudBackupFromPersistedState
import org.bitcoinppl.cove_core.AppInitException
import org.bitcoinppl.cove_core.CloudBackupEnableContext
import org.bitcoinppl.cove_core.CloudBackupOnboardingCompletionReadiness
import org.bitcoinppl.cove_core.CloudBackupRestoreFlow
import org.bitcoinppl.cove_core.CloudBackupVerificationSource
import org.bitcoinppl.cove_core.OnboardingBranch
import org.bitcoinppl.cove_core.OnboardingCloudRestoreState
import org.bitcoinppl.cove_core.OnboardingReconcileMessage
import org.bitcoinppl.cove_core.OnboardingRestoreState
import org.bitcoinppl.cove_core.OnboardingState
import org.bitcoinppl.cove_core.OnboardingStep
import org.bitcoinppl.cove_core.SavedPasskeyConfirmationMode
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class OnboardingHelpersTest {
    @Test
    fun resolveStartupModeUsesRustNeedsOnboardingDecision() {
        assertEquals(
            StartupMode.ONBOARDING,
            resolveStartupMode(needsOnboarding = true),
        )
        assertEquals(
            StartupMode.READY,
            resolveStartupMode(needsOnboarding = false),
        )
    }

    @Test
    fun databaseKeyMismatchUsesCatastrophicRecoveryStartupFailure() {
        val failure = classifyBootstrapFailure(AppInitException.DatabaseKeyMismatch("wrong key"))

        assertEquals(BootstrapFailure.CatastrophicRecovery, failure)
    }

    @Test
    fun nonCatastrophicBootstrapErrorsRemainFatal() {
        val failure = classifyBootstrapFailure(AppInitException.AlreadyCalled("already called"))

        assertEquals(
            BootstrapFailure.Fatal("App initialization error. Please force-quit and restart."),
            failure,
        )
    }

    @Test
    fun reduceOnboardingSnapshotAppliesStateUpdatesAndCompletion() {
        val initial =
            OnboardingSnapshot(
                state = defaultOnboardingState(),
                isComplete = false,
            )

        val withBranch =
            reduceOnboardingSnapshot(
                initial,
                OnboardingReconcileMessage.Branch(OnboardingBranch.EXCHANGE),
            )
        assertEquals(OnboardingBranch.EXCHANGE, withBranch.state.branch)

        val withWords =
            reduceOnboardingSnapshot(
                withBranch,
                OnboardingReconcileMessage.CreatedWords(listOf("alpha", "beta")),
            )
        assertEquals(listOf("alpha", "beta"), withWords.state.createdWords)

        val complete =
            reduceOnboardingSnapshot(
                withWords,
                OnboardingReconcileMessage.Complete,
            )
        assertTrue(complete.isComplete)
    }

    @Test
    fun reduceOnboardingSnapshotAppliesRestoreStateUpdates() {
        val failedState = OnboardingRestoreState.Failed("restore failed")
        val initial =
            OnboardingSnapshot(
                state = defaultOnboardingState(),
                isComplete = false,
            )

        val updated =
            reduceOnboardingSnapshot(
                initial,
                OnboardingReconcileMessage.RestoreStateChanged(failedState),
            )

        assertEquals(failedState, updated.state.restoreState)
        assertFalse(updated.isComplete)
    }

    @Test
    fun combinedRestoreProgressTracksDownloadAndRestoreStages() {
        val downloading =
            OnboardingRestoreState.Restoring(
                CloudBackupRestoreFlow.Downloading(completed = 2u, total = 4u),
            )
        val restoring =
            OnboardingRestoreState.Restoring(
                CloudBackupRestoreFlow.Restoring(completed = 1u, total = 4u),
            )

        assertEquals(0.25f, combinedRestoreProgress(downloading), 0.0001f)
        assertEquals(0.625f, combinedRestoreProgress(restoring), 0.0001f)
    }

    @Test
    fun onboardingCloudBackupCompletionUsesEventContextAndPersistedFallback() {
        assertTrue(
            isOnboardingCloudBackupEnableCompletion(
                CloudBackupEnableContext(
                    SavedPasskeyConfirmationMode.AUTOMATIC,
                    CloudBackupVerificationSource.ONBOARDING,
                ),
            ),
        )
        assertFalse(
            isOnboardingCloudBackupEnableCompletion(
                CloudBackupEnableContext(
                    SavedPasskeyConfirmationMode.MANUAL,
                    CloudBackupVerificationSource.SETTINGS,
                ),
            ),
        )

        assertTrue(
            shouldCompleteOnboardingCloudBackupFromPersistedState(
                CloudBackupOnboardingCompletionReadiness.READY,
            ),
        )
        assertFalse(
            shouldCompleteOnboardingCloudBackupFromPersistedState(
                CloudBackupOnboardingCompletionReadiness.NOT_READY,
            ),
        )
        assertFalse(
            shouldCompleteOnboardingCloudBackupFromPersistedState(
                CloudBackupOnboardingCompletionReadiness.PENDING_ENABLE_RECOVERY,
            ),
        )
    }

    private fun defaultOnboardingState() =
        OnboardingState(
            step = OnboardingStep.TERMS,
            branch = null,
            createdWords = emptyList(),
            cloudBackupEnabled = false,
            secretWordsSaved = false,
            cloudRestoreState = OnboardingCloudRestoreState.CHECKING,
            cloudRestoreMessage = null,
            cloudRestoreProviderHint = null,
            shouldOfferCloudRestore = false,
            cloudRestoreAlertVisible = false,
            restoreState = OnboardingRestoreState.Idle,
            errorMessage = null,
        )
}
