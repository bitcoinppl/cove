package org.bitcoinppl.cove.flows.OnboardingFlow

import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import org.bitcoinppl.cove.OnboardingManager
import org.bitcoinppl.cove_core.OnboardingAction
import org.bitcoinppl.cove_core.OnboardingCloudRestoreState
import org.bitcoinppl.cove_core.OnboardingReturningUserSelection
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

    val onOpenCloudRestore =
        if (manager.state.cloudRestoreState != OnboardingCloudRestoreState.NO_BACKUP_FOUND) {
            { manager.dispatch(OnboardingAction.OpenCloudRestore) }
        } else {
            null
        }

    val restoreWarningMessage =
        if (manager.state.step == OnboardingStep.RESTORE_OFFER &&
            manager.state.cloudRestoreState == OnboardingCloudRestoreState.INCONCLUSIVE
        ) {
            manager.state.cloudRestoreMessage
        } else {
            null
        }

    val stepContent: Unit = when (manager.state.step) {
        OnboardingStep.TERMS ->
            OnboardingTermsScreen(
                errorMessage = manager.state.errorMessage,
                onAgree = {
                    manager.app.agreeToTerms()
                    manager.dispatch(OnboardingAction.AcceptTerms)
                },
            )

        OnboardingStep.CLOUD_CHECK -> CloudCheckContent()

        OnboardingStep.RESTORE_OFFER ->
            OnboardingRestoreOfferView(
                warningMessage = restoreWarningMessage,
                errorMessage = manager.state.errorMessage,
                onRestore = { manager.dispatch(OnboardingAction.StartRestore) },
                onSkip = { manager.dispatch(OnboardingAction.SkipRestore) },
            )

        OnboardingStep.RESTORE_OFFLINE ->
            OnboardingRestoreOfflineScreen(
                onContinue = { manager.dispatch(OnboardingAction.ContinueWithoutCloudRestore) },
                onBack = { manager.dispatch(OnboardingAction.Back) },
            )

        OnboardingStep.RESTORE_UNAVAILABLE ->
            OnboardingRestoreUnavailableScreen(
                onContinue = { manager.dispatch(OnboardingAction.ContinueWithoutCloudRestore) },
                onBack = { manager.dispatch(OnboardingAction.Back) },
            )

        OnboardingStep.RESTORING ->
            OnboardingRestoreView(
                onComplete = { manager.dispatch(OnboardingAction.RestoreComplete) },
                onError = { error -> manager.dispatch(OnboardingAction.RestoreFailed(error)) },
            )

        OnboardingStep.WELCOME ->
            OnboardingWelcomeScreen(
                errorMessage = manager.state.errorMessage,
                onContinue = { manager.dispatch(OnboardingAction.ContinueFromWelcome) },
            )

        OnboardingStep.BITCOIN_CHOICE ->
            OnboardingBitcoinChoiceScreen(
                errorMessage = manager.state.errorMessage,
                onNewHere = { manager.dispatch(OnboardingAction.SelectHasBitcoin(false)) },
                onHasBitcoin = { manager.dispatch(OnboardingAction.SelectHasBitcoin(true)) },
            )

        OnboardingStep.RETURNING_USER_CHOICE ->
            OnboardingReturningUserChoiceScreen(
                onRestoreFromCoveBackup = {
                    manager.dispatch(
                        OnboardingAction.SelectReturningUserFlow(
                            OnboardingReturningUserSelection.RESTORE_FROM_COVE_BACKUP,
                        ),
                    )
                },
                onUseAnotherWallet = {
                    manager.dispatch(
                        OnboardingAction.SelectReturningUserFlow(
                            OnboardingReturningUserSelection.USE_ANOTHER_WALLET,
                        ),
                    )
                },
                onBack = { manager.dispatch(OnboardingAction.Back) },
            )

        OnboardingStep.STORAGE_CHOICE ->
            OnboardingStorageChoiceScreen(
                errorMessage = manager.state.errorMessage,
                onRestoreFromCoveBackup = onOpenCloudRestore,
                onSelectStorage = { selection ->
                    manager.dispatch(OnboardingAction.SelectStorage(selection))
                },
                onBack = { manager.dispatch(OnboardingAction.Back) },
            )

        OnboardingStep.SOFTWARE_CHOICE ->
            OnboardingSoftwareChoiceScreen(
                errorMessage = manager.state.errorMessage,
                onRestoreFromCoveBackup = onOpenCloudRestore,
                onSelectSoftwareAction = { selection ->
                    manager.dispatch(OnboardingAction.SelectSoftwareAction(selection))
                },
                onBack = { manager.dispatch(OnboardingAction.Back) },
            )

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
                onEnabled = { manager.dispatch(OnboardingAction.CloudBackupEnabled) },
                onSkip = { manager.dispatch(OnboardingAction.SkipCloudBackup) },
            )

        OnboardingStep.SECRET_WORDS ->
            OnboardingSecretWordsView(
                words = manager.state.createdWords,
                onBack = { manager.dispatch(OnboardingAction.Back) },
                onSaved = { manager.dispatch(OnboardingAction.SecretWordsSaved) },
            )

        OnboardingStep.EXCHANGE_FUNDING ->
            OnboardingExchangeFundingView(
                app = manager.app,
                manager = manager,
                onContinue = { manager.dispatch(OnboardingAction.ContinueFromExchangeFunding) },
            )

        OnboardingStep.HARDWARE_IMPORT ->
            OnboardingHardwareImportFlowView(
                onImported = { walletId ->
                    manager.dispatch(OnboardingAction.HardwareImportCompleted(walletId))
                },
                onBack = { manager.dispatch(OnboardingAction.Back) },
            )

        OnboardingStep.SOFTWARE_IMPORT ->
            OnboardingSoftwareImportFlowView(
                onImported = { walletId ->
                    manager.dispatch(OnboardingAction.SoftwareImportCompleted(walletId))
                },
                onBack = { manager.dispatch(OnboardingAction.Back) },
            )
    }
    stepContent
}
