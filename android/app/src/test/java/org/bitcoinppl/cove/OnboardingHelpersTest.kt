package org.bitcoinppl.cove

import org.bitcoinppl.cove.flows.OnboardingFlow.OnboardingRestorePhase
import org.bitcoinppl.cove.flows.OnboardingFlow.combinedRestoreProgress
import org.bitcoinppl.cove.flows.OnboardingFlow.resolveRestorePhase
import org.bitcoinppl.cove.flows.OnboardingFlow.shouldCompleteOnboardingCloudBackup
import org.bitcoinppl.cove.flows.OnboardingFlow.shouldNotifyRestoreError
import org.bitcoinppl.cove_core.AppInitException
import org.bitcoinppl.cove_core.CloudBackupConfiguredState
import org.bitcoinppl.cove_core.CloudBackupDetailState
import org.bitcoinppl.cove_core.CloudBackupFailure
import org.bitcoinppl.cove_core.CloudBackupLifecycle
import org.bitcoinppl.cove_core.CloudBackupPasskeyState
import org.bitcoinppl.cove_core.CloudBackupRestoreProgress
import org.bitcoinppl.cove_core.CloudBackupRestoreReport
import org.bitcoinppl.cove_core.CloudBackupRestoreStage
import org.bitcoinppl.cove_core.CloudBackupRootPrompt
import org.bitcoinppl.cove_core.CloudBackupSyncState
import org.bitcoinppl.cove_core.CloudBackupVerificationPresentation
import org.bitcoinppl.cove_core.CloudBackupVerificationState
import org.bitcoinppl.cove_core.DeepVerificationFailure
import org.bitcoinppl.cove_core.DeepVerificationReport
import org.bitcoinppl.cove_core.OnboardingBranch
import org.bitcoinppl.cove_core.OnboardingCloudRestoreState
import org.bitcoinppl.cove_core.OnboardingReconcileMessage
import org.bitcoinppl.cove_core.OnboardingState
import org.bitcoinppl.cove_core.OnboardingStep
import org.bitcoinppl.cove_core.device.CloudSyncHealth
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class OnboardingHelpersTest {
    @Test
    fun resolveStartupModeMirrorsIosStartupShell() {
        assertEquals(
            StartupMode.ONBOARDING,
            resolveStartupMode(
                termsAccepted = false,
                hasWallets = false,
                cloudBackupLifecycle = CloudBackupLifecycle.Disabled,
                hasPersistedOnboardingProgress = false,
            ),
        )
        assertEquals(
            StartupMode.ONBOARDING,
            resolveStartupMode(
                termsAccepted = false,
                hasWallets = true,
                cloudBackupLifecycle = configuredLifecycle(),
                hasPersistedOnboardingProgress = false,
            ),
        )
        assertEquals(
            StartupMode.ONBOARDING,
            resolveStartupMode(
                termsAccepted = true,
                hasWallets = false,
                cloudBackupLifecycle = CloudBackupLifecycle.Disabled,
                hasPersistedOnboardingProgress = false,
            ),
        )
        assertEquals(
            StartupMode.READY,
            resolveStartupMode(
                termsAccepted = true,
                hasWallets = false,
                cloudBackupLifecycle = configuredLifecycle(),
                hasPersistedOnboardingProgress = false,
            ),
        )
        assertEquals(
            StartupMode.READY,
            resolveStartupMode(
                termsAccepted = true,
                hasWallets = false,
                cloudBackupLifecycle = configuredLifecycle(passkey = CloudBackupPasskeyState.Missing),
                hasPersistedOnboardingProgress = false,
            ),
        )
        assertEquals(
            StartupMode.READY,
            resolveStartupMode(
                termsAccepted = true,
                hasWallets = true,
                cloudBackupLifecycle = CloudBackupLifecycle.Disabled,
                hasPersistedOnboardingProgress = false,
            ),
        )
        assertEquals(
            StartupMode.ONBOARDING,
            resolveStartupMode(
                termsAccepted = true,
                hasWallets = true,
                cloudBackupLifecycle = configuredLifecycle(),
                hasPersistedOnboardingProgress = true,
            ),
        )
    }

    @Test
    fun persistedOnboardingProgressRequiresNonBlankState() {
        assertFalse(hasPersistedOnboardingProgress(null))
        assertFalse(hasPersistedOnboardingProgress(""))
        assertFalse(hasPersistedOnboardingProgress("   "))
        assertTrue(hasPersistedOnboardingProgress("""{"step":"backup_wallet"}"""))
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
                lifecycle = configuredLifecycle(),
                restoreReport = report,
                currentPhase = OnboardingRestorePhase.Restoring,
            )
        assertTrue(completePhase is OnboardingRestorePhase.Complete)

        val errorPhase =
            resolveRestorePhase(
                lifecycle =
                    CloudBackupLifecycle.Failed(
                        CloudBackupFailure(
                            message = "restore failed",
                            restoreReport = null,
                        ),
                    ),
                restoreReport = null,
                currentPhase = OnboardingRestorePhase.Restoring,
            )
        assertTrue(errorPhase is OnboardingRestorePhase.Error)
        assertEquals("restore failed", (errorPhase as OnboardingRestorePhase.Error).message)
    }

    @Test
    fun shouldNotifyRestoreErrorOnlyForFirstRestoringError() {
        assertTrue(shouldNotifyRestoreError(OnboardingRestorePhase.Restoring, hasDeliveredError = false))
        assertFalse(shouldNotifyRestoreError(OnboardingRestorePhase.Restoring, hasDeliveredError = true))
        assertFalse(
            shouldNotifyRestoreError(
                currentPhase = OnboardingRestorePhase.Error("restore failed"),
                hasDeliveredError = false,
            ),
        )
    }

    @Test
    fun shouldCompleteOnboardingCloudBackupRequiresCompletedVerification() {
        assertTrue(
            shouldCompleteOnboardingCloudBackup(
                configuredState =
                    configuredState(
                        verification =
                            CloudBackupVerificationState.Verified(
                                report = defaultVerificationReport(),
                                lastVerifiedAt = null,
                            ),
                    ),
                hasPendingUploadVerification = false,
            ),
        )
        assertFalse(
            shouldCompleteOnboardingCloudBackup(
                configuredState = null,
                hasPendingUploadVerification = false,
            ),
        )
        assertFalse(
            shouldCompleteOnboardingCloudBackup(
                configuredState =
                    configuredState(
                        verification =
                            CloudBackupVerificationState.Verified(
                                report = defaultVerificationReport(),
                                lastVerifiedAt = null,
                            ),
                    ),
                hasPendingUploadVerification = true,
            ),
        )
        assertFalse(
            shouldCompleteOnboardingCloudBackup(
                configuredState = configuredState(verification = CloudBackupVerificationState.NotVerified),
                hasPendingUploadVerification = false,
            ),
        )
        assertFalse(
            shouldCompleteOnboardingCloudBackup(
                configuredState =
                    configuredState(
                        verification =
                            CloudBackupVerificationState.Failed(
                                DeepVerificationFailure.Retry("verification failed", null, null),
                            ),
                    ),
                hasPendingUploadVerification = false,
            ),
        )
    }

    private fun configuredLifecycle(
        passkey: CloudBackupPasskeyState = CloudBackupPasskeyState.Available,
        verification: CloudBackupVerificationState = CloudBackupVerificationState.NotVerified,
    ): CloudBackupLifecycle =
        CloudBackupLifecycle.Configured(
            configuredState(
                passkey = passkey,
                verification = verification,
            ),
        )

    private fun configuredState(
        passkey: CloudBackupPasskeyState = CloudBackupPasskeyState.Available,
        verification: CloudBackupVerificationState = CloudBackupVerificationState.NotVerified,
        sync: CloudBackupSyncState = CloudBackupSyncState.Idle,
    ): CloudBackupConfiguredState =
        CloudBackupConfiguredState(
            passkey = passkey,
            verification = verification,
            sync = sync,
            detail = CloudBackupDetailState.NotLoaded,
            lastRestoreReport = null,
            rootPrompt = CloudBackupRootPrompt.None,
            syncHealth = CloudSyncHealth.Unknown,
            verificationPresentation = CloudBackupVerificationPresentation.Hidden(null),
        )

    private fun defaultVerificationReport() =
        DeepVerificationReport(
            masterKeyWrapperRepaired = false,
            localMasterKeyRepaired = false,
            credentialRecovered = false,
            walletsVerified = 0U,
            walletsFailed = 0U,
            walletsUnsupported = 0U,
            detail = null,
        )

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
            errorMessage = null,
        )
}
