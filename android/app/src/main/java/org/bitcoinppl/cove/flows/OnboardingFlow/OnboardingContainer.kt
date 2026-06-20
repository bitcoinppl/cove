package org.bitcoinppl.cove.flows.OnboardingFlow

import androidx.activity.compose.BackHandler
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import org.bitcoinppl.cove.OnboardingManager
import org.bitcoinppl.cove.localizedMessage
import org.bitcoinppl.cove_core.OnboardingAction
import org.bitcoinppl.cove_core.OnboardingCloudRestoreState
import org.bitcoinppl.cove_core.OnboardingStep

@Composable
internal fun OnboardingContainer(
    manager: OnboardingManager,
    onComplete: () -> Unit,
) {
    LaunchedEffect(manager.isComplete) {
        if (!manager.isComplete) return@LaunchedEffect
        manager.app.loadWallets()
        onComplete()
    }

    val restoreWarningMessage =
        if (manager.state.step == OnboardingStep.RESTORE_OFFER &&
            manager.state.cloudRestoreState == OnboardingCloudRestoreState.INCONCLUSIVE
        ) {
            manager.state.cloudRestoreIssue?.localizedMessage()?.asString()
        } else {
            null
        }

    val stepContent: Unit = when (manager.state.step) {
        OnboardingStep.TERMS ->
            OnboardingTermsScreen(
                errorMessage = manager.state.errorMessage,
                onAgree = {
                    manager.dispatch(OnboardingAction.AcceptTerms)
                },
            )

        OnboardingStep.CLOUD_CHECK -> CloudCheckContent()

        OnboardingStep.RESTORE_OFFER ->
            BackableOnboardingStep(manager) {
                OnboardingRestoreOfferView(
                    warningMessage = restoreWarningMessage,
                    errorMessage = manager.state.errorMessage,
                    providerHint = manager.state.cloudRestoreProviderHint,
                    onBack = { manager.dispatch(OnboardingAction.Back) },
                    onRestore = { manager.dispatch(OnboardingAction.StartRestore) },
                    onSkip = { manager.dispatch(OnboardingAction.SkipRestore) },
                )
            }

        OnboardingStep.RESTORE_OFFLINE ->
            BackableOnboardingStep(manager) {
                OnboardingRestoreOfflineScreen(
                    onContinue = { manager.dispatch(OnboardingAction.ContinueWithoutCloudRestore) },
                    onBack = { manager.dispatch(OnboardingAction.Back) },
                )
            }

        OnboardingStep.RESTORE_UNAVAILABLE ->
            BackableOnboardingStep(manager) {
                OnboardingRestoreUnavailableScreen(
                    onContinue = { manager.dispatch(OnboardingAction.ContinueWithoutCloudRestore) },
                    onBack = { manager.dispatch(OnboardingAction.Back) },
                )
            }

        OnboardingStep.RESTORING,
        OnboardingStep.RESTORE_COMPLETE,
        OnboardingStep.RESTORE_FAILED ->
            OnboardingRestoreView(
                restoreState = manager.state.restoreState,
                onDone = { manager.dispatch(OnboardingAction.ContinueFromRestoreComplete) },
                onRetry = { manager.dispatch(OnboardingAction.RetryRestore) },
                onContinueWithoutBackup = { manager.dispatch(OnboardingAction.SkipRestore) },
            )

        OnboardingStep.WELCOME ->
            OnboardingWelcomeScreen(
                errorMessage = manager.state.errorMessage,
                onContinue = { manager.dispatch(OnboardingAction.ContinueFromWelcome) },
            )

        OnboardingStep.BITCOIN_CHOICE ->
            OnboardingBitcoinChoiceScreen(
                errorMessage = manager.state.errorMessage,
                onRestoreFromCoveBackup = { manager.dispatch(OnboardingAction.OpenCloudRestore) },
                onNewHere = { manager.dispatch(OnboardingAction.SelectHasBitcoin(false)) },
                onHasBitcoin = { manager.dispatch(OnboardingAction.SelectHasBitcoin(true)) },
            )

        OnboardingStep.STORAGE_CHOICE ->
            BackableOnboardingStep(manager) {
                OnboardingStorageChoiceScreen(
                    errorMessage = manager.state.errorMessage,
                    onRestoreFromCoveBackup = { manager.dispatch(OnboardingAction.OpenCloudRestore) },
                    onSelectStorage = { selection ->
                        manager.dispatch(OnboardingAction.SelectStorage(selection))
                    },
                    onBack = { manager.dispatch(OnboardingAction.Back) },
                )
            }

        OnboardingStep.CREATING_WALLET ->
            OnboardingCreatingWalletView(
                onContinue = { manager.dispatch(OnboardingAction.ContinueWalletCreation) },
            )

        OnboardingStep.BACKUP_WALLET ->
            OnboardingBackupWalletView(
                branch = manager.state.branch,
                secretWordsSaved = manager.state.secretWordsSaved,
                cloudBackupEnabled = manager.state.cloudBackupEnabled,
                wordCount = manager.state.createdWords.size,
                onShowWords = { manager.dispatch(OnboardingAction.ShowSecretWords) },
                onEnableCloudBackup = { manager.dispatch(OnboardingAction.OpenCloudBackup) },
                onContinue = { manager.dispatch(OnboardingAction.ContinueFromBackup) },
            )

        OnboardingStep.CLOUD_BACKUP ->
            OnboardingCloudBackupStepView(
                branch = manager.state.branch,
                onEnable = { manager.beginCloudBackupEnable() },
                onEnabled = { manager.dispatch(OnboardingAction.CloudBackupEnabled) },
                onSkip = { manager.dispatch(OnboardingAction.SkipCloudBackup) },
            )

        OnboardingStep.CLOUD_BACKUP_SUCCESS ->
            OnboardingCloudBackupSuccessView(
                onContinue = { manager.dispatch(OnboardingAction.ContinueFromCloudBackupSuccess) },
            )

        OnboardingStep.SECRET_WORDS ->
            BackableOnboardingStep(manager) {
                OnboardingSecretWordsView(
                    words = manager.state.createdWords,
                    onBack = { manager.dispatch(OnboardingAction.Back) },
                    onSaved = { manager.dispatch(OnboardingAction.SecretWordsSaved) },
                )
            }

        OnboardingStep.EXCHANGE_FUNDING ->
            OnboardingExchangeFundingView(
                app = manager.app,
                manager = manager,
                onContinue = { manager.dispatch(OnboardingAction.ContinueFromExchangeFunding) },
            )

        OnboardingStep.HARDWARE_IMPORT ->
            OnboardingHardwareImportFlowView(
                cloudRestoreAlertVisible = manager.state.cloudRestoreAlertVisible,
                onImported = { walletId ->
                    manager.dispatch(OnboardingAction.HardwareImportCompleted(walletId))
                },
                onRestoreFromCloudBackup = { manager.dispatch(OnboardingAction.OpenCloudRestore) },
                onDismissCloudRestoreAlert = {
                    manager.dispatch(OnboardingAction.DismissCloudRestoreAlert)
                },
                onBack = { manager.dispatch(OnboardingAction.Back) },
            )

        OnboardingStep.SOFTWARE_IMPORT ->
            OnboardingSoftwareImportFlowView(
                errorMessage = manager.state.errorMessage,
                cloudRestoreAlertVisible = manager.state.cloudRestoreAlertVisible,
                onImported = { walletId ->
                    manager.dispatch(OnboardingAction.SoftwareImportCompleted(walletId))
                },
                onCreateWallet = { manager.dispatch(OnboardingAction.CreateSoftwareWallet) },
                onRestoreFromCloudBackup = { manager.dispatch(OnboardingAction.OpenCloudRestore) },
                onDismissCloudRestoreAlert = {
                    manager.dispatch(OnboardingAction.DismissCloudRestoreAlert)
                },
                onBack = { manager.dispatch(OnboardingAction.Back) },
            )
    }
    stepContent
}

@Composable
private fun BackableOnboardingStep(
    manager: OnboardingManager,
    content: @Composable () -> Unit,
) {
    BackHandler {
        manager.dispatch(OnboardingAction.Back)
    }

    content()
}

private fun OnboardingManager.beginCloudBackupEnable() {
    dispatch(OnboardingAction.BeginCloudBackupEnable)
}
