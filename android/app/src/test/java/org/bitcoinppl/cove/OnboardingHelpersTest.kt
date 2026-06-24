package org.bitcoinppl.cove

import org.bitcoinppl.cove.flows.OnboardingFlow.combinedRestoreProgress
import org.bitcoinppl.cove.flows.OnboardingFlow.shouldCompleteOnboardingCloudBackup
import org.bitcoinppl.cove_core.AppInitException
import org.bitcoinppl.cove_core.CloudBackupConfiguredState
import org.bitcoinppl.cove_core.CloudBackupDetailState
import org.bitcoinppl.cove_core.CloudBackupDestructiveOperationState
import org.bitcoinppl.cove_core.CloudBackupLifecycle
import org.bitcoinppl.cove_core.CloudBackupPasskeyRepairState
import org.bitcoinppl.cove_core.CloudBackupPasskeyState
import org.bitcoinppl.cove_core.CloudBackupRestoreFlow
import org.bitcoinppl.cove_core.CloudBackupRootPrompt
import org.bitcoinppl.cove_core.CloudBackupSyncState
import org.bitcoinppl.cove_core.CloudBackupVerificationPresentation
import org.bitcoinppl.cove_core.CloudBackupVerificationState
import org.bitcoinppl.cove_core.DeepVerificationFailure
import org.bitcoinppl.cove_core.DeepVerificationReport
import org.bitcoinppl.cove_core.OnboardingBranch
import org.bitcoinppl.cove_core.OnboardingCloudRestoreState
import org.bitcoinppl.cove_core.OnboardingReconcileMessage
import org.bitcoinppl.cove_core.OnboardingRestoreState
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
    fun readyStartupModeDoesNotReenterOnboardingWhenLastWalletIsDeleted() {
        assertEquals(
            StartupMode.READY,
            resolveStartupModeTransition(
                currentMode = StartupMode.READY,
                termsAccepted = true,
                hasWallets = false,
                cloudBackupLifecycle = CloudBackupLifecycle.Disabled,
                hasPersistedOnboardingProgress = false,
            ),
        )
    }

    @Test
    fun onboardingStartupModePromotesToReadyWhenSetupIsComplete() {
        assertEquals(
            StartupMode.READY,
            resolveStartupModeTransition(
                currentMode = StartupMode.ONBOARDING,
                termsAccepted = true,
                hasWallets = true,
                cloudBackupLifecycle = configuredLifecycle(),
                hasPersistedOnboardingProgress = false,
            ),
        )
    }

    @Test
    fun onboardingStartupModeKeepsPersistedProgressEvenWithWallets() {
        assertEquals(
            StartupMode.ONBOARDING,
            resolveStartupModeTransition(
                currentMode = StartupMode.ONBOARDING,
                termsAccepted = true,
                hasWallets = true,
                cloudBackupLifecycle = configuredLifecycle(),
                hasPersistedOnboardingProgress = true,
            ),
        )
    }

    @Test
    fun onboardingStartupModeDoesNotUseReadyShortcut() {
        assertEquals(
            StartupMode.ONBOARDING,
            resolveStartupModeTransition(
                currentMode = StartupMode.ONBOARDING,
                termsAccepted = true,
                hasWallets = false,
                cloudBackupLifecycle = CloudBackupLifecycle.Disabled,
                hasPersistedOnboardingProgress = false,
            ),
        )
    }

    @Test
    fun readyStartupModeIgnoresStalePersistedOnboardingProgress() {
        val freshProgress = Result.success("""{"step":"backup_wallet"}""")
        val recoveredProgress =
            hasRecoveredOnboardingProgressAfterReadFailure(
                freshProgress = freshProgress,
                previousProgress = null,
                previousReadFailed = false,
            )

        assertFalse(recoveredProgress)
        assertEquals(
            StartupMode.READY,
            resolveStartupModeTransition(
                currentMode = StartupMode.READY,
                termsAccepted = true,
                hasWallets = false,
                cloudBackupLifecycle = CloudBackupLifecycle.Disabled,
                hasPersistedOnboardingProgress = true,
                hasRecoveredOnboardingProgressAfterReadFailure = recoveredProgress,
            ),
        )
    }

    @Test
    fun readyStartupModeReevaluatesRecoveredPersistedOnboardingProgress() {
        val freshProgress = Result.success("""{"step":"backup_wallet"}""")
        val recoveredProgress =
            hasRecoveredOnboardingProgressAfterReadFailure(
                freshProgress = freshProgress,
                previousProgress = null,
                previousReadFailed = true,
            )

        assertTrue(recoveredProgress)
        assertEquals(
            StartupMode.ONBOARDING,
            resolveStartupModeTransition(
                currentMode = StartupMode.READY,
                termsAccepted = true,
                hasWallets = true,
                cloudBackupLifecycle = configuredLifecycle(),
                hasPersistedOnboardingProgress = hasPersistedOnboardingProgress(freshProgress.getOrNull()),
                hasRecoveredOnboardingProgressAfterReadFailure = recoveredProgress,
            ),
        )
    }

    @Test
    fun readyStartupModeStillRequiresAcceptedTerms() {
        assertEquals(
            StartupMode.ONBOARDING,
            resolveStartupModeTransition(
                currentMode = StartupMode.READY,
                termsAccepted = false,
                hasWallets = true,
                cloudBackupLifecycle = configuredLifecycle(),
                hasPersistedOnboardingProgress = false,
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
    fun effectiveOnboardingProgressUsesFreshValueOnSuccess() {
        assertEquals(
            """{"step":"backup_wallet"}""",
            resolveEffectiveOnboardingProgress(
                freshProgress = Result.success("""{"step":"backup_wallet"}"""),
                previousProgress = """{"step":"terms"}""",
            ),
        )
    }

    @Test
    fun effectiveOnboardingProgressClearsPreviousValueOnSuccessfulNull() {
        assertEquals(
            null,
            resolveEffectiveOnboardingProgress(
                freshProgress = Result.success(null),
                previousProgress = """{"step":"backup_wallet"}""",
            ),
        )
    }

    @Test
    fun effectiveOnboardingProgressKeepsPreviousValueOnFailure() {
        assertEquals(
            """{"step":"backup_wallet"}""",
            resolveEffectiveOnboardingProgress(
                freshProgress = Result.failure(IllegalStateException("read failed")),
                previousProgress = """{"step":"backup_wallet"}""",
            ),
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
                configuredState =
                    configuredState(
                        passkey = CloudBackupPasskeyState.Missing,
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
                configuredState =
                    configuredState(
                        passkey =
                            CloudBackupPasskeyState.NeedsRepair(
                                CloudBackupPasskeyRepairState.Idle,
                            ),
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
                configuredState =
                    configuredState(
                        verification =
                            CloudBackupVerificationState.Verified(
                                report = null,
                                lastVerifiedAt = null,
                            ),
                    ),
                hasPendingUploadVerification = false,
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
            destructiveOperation = CloudBackupDestructiveOperationState.Idle,
            detail = CloudBackupDetailState.NotLoaded,
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
            restoreState = OnboardingRestoreState.Idle,
            errorMessage = null,
        )
}
