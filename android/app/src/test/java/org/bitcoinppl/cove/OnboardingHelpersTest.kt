package org.bitcoinppl.cove

import org.bitcoinppl.cove.flows.OnboardingFlow.OnboardingRestorePhase
import org.bitcoinppl.cove.flows.OnboardingFlow.combinedRestoreProgress
import org.bitcoinppl.cove.flows.OnboardingFlow.resolveRestorePhase
import org.bitcoinppl.cove.flows.OnboardingFlow.shouldCompleteOnboardingCloudBackup
import org.bitcoinppl.cove_core.CloudBackupRestoreProgress
import org.bitcoinppl.cove_core.CloudBackupRestoreReport
import org.bitcoinppl.cove_core.CloudBackupRestoreStage
import org.bitcoinppl.cove_core.CloudBackupStatus
import org.bitcoinppl.cove_core.OnboardingBranch
import org.bitcoinppl.cove_core.OnboardingCloudRestoreState
import org.bitcoinppl.cove_core.OnboardingReconcileMessage
import org.bitcoinppl.cove_core.OnboardingState
import org.bitcoinppl.cove_core.OnboardingStep
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class OnboardingHelpersTest {
    @Test
    fun shouldStartOnboardingRequiresAcceptedTermsAndWallets() {
        assertTrue(shouldStartOnboarding(termsAccepted = false, hasWallets = false))
        assertTrue(shouldStartOnboarding(termsAccepted = false, hasWallets = true))
        assertTrue(shouldStartOnboarding(termsAccepted = true, hasWallets = false))
        assertFalse(shouldStartOnboarding(termsAccepted = true, hasWallets = true))
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
    fun combinedRestoreProgressTracksDownloadAndRestoreStages() {
        val downloading =
            CloudBackupRestoreProgress(
                stage = CloudBackupRestoreStage.DOWNLOADING,
                completed = 2u,
                total = 4u,
            )
        val restoring =
            CloudBackupRestoreProgress(
                stage = CloudBackupRestoreStage.RESTORING,
                completed = 1u,
                total = 4u,
            )

        assertEquals(0.25f, combinedRestoreProgress(downloading), 0.0001f)
        assertEquals(0.625f, combinedRestoreProgress(restoring), 0.0001f)
    }

    @Test
    fun resolveRestorePhasePromotesCompletionAndErrors() {
        val report =
            CloudBackupRestoreReport(
                walletsRestored = 1u,
                walletsFailed = 0u,
                failedWalletErrors = emptyList(),
                labelsFailedWalletNames = emptyList(),
                labelsFailedErrors = emptyList(),
            )

        val completePhase =
            resolveRestorePhase(
                status = CloudBackupStatus.Enabled,
                restoreReport = report,
                currentPhase = OnboardingRestorePhase.Restoring,
            )
        assertTrue(completePhase is OnboardingRestorePhase.Complete)

        val errorPhase =
            resolveRestorePhase(
                status = CloudBackupStatus.Error("restore failed"),
                restoreReport = null,
                currentPhase = OnboardingRestorePhase.Restoring,
            )
        assertTrue(errorPhase is OnboardingRestorePhase.Error)
        assertEquals("restore failed", (errorPhase as OnboardingRestorePhase.Error).message)
    }

    @Test
    fun shouldCompleteOnboardingCloudBackupAcceptsEnabledAndConfiguredFallback() {
        assertTrue(
            shouldCompleteOnboardingCloudBackup(
                status = CloudBackupStatus.Enabled,
                isCloudBackupEnabled = false,
                isConfigured = false,
            ),
        )
        assertTrue(
            shouldCompleteOnboardingCloudBackup(
                status = CloudBackupStatus.Enabling,
                isCloudBackupEnabled = true,
                isConfigured = true,
            ),
        )
        assertFalse(
            shouldCompleteOnboardingCloudBackup(
                status = CloudBackupStatus.Disabled,
                isCloudBackupEnabled = true,
                isConfigured = false,
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
            shouldOfferCloudRestore = false,
            errorMessage = null,
        )
}
