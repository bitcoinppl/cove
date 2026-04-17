import SwiftUI

struct OnboardingContainer: View {
    let manager: OnboardingManager
    let auth: AuthManager
    let onComplete: () -> Void

    var body: some View {
        CloudBackupPresentationHost(
            app: manager.app,
            auth: auth,
            isCoverPresented: false
        ) {
            stepView(for: manager.state.step)
                .onChange(of: manager.isComplete) { _, complete in
                    guard complete else { return }
                    manager.app.reloadWallets()
                    onComplete()
                }
        }
        .environment(manager.app)
    }

    private var onOpenCloudRestore: (() -> Void)? {
        guard manager.state.shouldOfferCloudRestore else { return nil }
        return {
            manager.dispatch(.openCloudRestore)
        }
    }

    private var restoreWarningMessage: String? {
        guard manager.state.step == .restoreOffer,
              manager.state.cloudRestoreState == .inconclusive
        else { return nil }

        return manager.state.cloudRestoreMessage
    }

    @ViewBuilder
    private func stepView(for step: OnboardingStep) -> some View {
        switch step {
        case .terms:
            TermsAndConditionsView(errorMessage: manager.state.errorMessage) {
                manager.dispatch(.acceptTerms)
            }

        case .cloudCheck:
            CloudCheckContent()

        case .restoreOffer:
            CloudRestoreOfferView(
                onRestore: {
                    manager.dispatch(.startRestore)
                },
                onSkip: {
                    manager.dispatch(.skipRestore)
                },
                warningMessage: restoreWarningMessage,
                errorMessage: manager.state.errorMessage
            )

        case .restoreUnavailable:
            OnboardingRestoreUnavailableScreen(
                onContinue: { manager.dispatch(.continueWithoutCloudRestore) },
                onBack: { manager.dispatch(.back) }
            )

        case .restoring:
            DeviceRestoreView(
                onComplete: { manager.dispatch(.restoreComplete) },
                onError: { error in manager.dispatch(.restoreFailed(error: error)) }
            )

        case .welcome:
            OnboardingWelcomeScreen(errorMessage: manager.state.errorMessage) {
                manager.dispatch(.continueFromWelcome)
            }

        case .bitcoinChoice:
            OnboardingBitcoinChoiceScreen(
                errorMessage: manager.state.errorMessage,
                onNewHere: { manager.dispatch(.selectHasBitcoin(hasBitcoin: false)) },
                onHasBitcoin: { manager.dispatch(.selectHasBitcoin(hasBitcoin: true)) }
            )

        case .returningUserChoice:
            OnboardingReturningUserChoiceScreen(
                onRestoreFromCoveBackup: {
                    manager.dispatch(
                        .selectReturningUserFlow(selection: .restoreFromCoveBackup)
                    )
                },
                onUseAnotherWallet: {
                    manager.dispatch(.selectReturningUserFlow(selection: .useAnotherWallet))
                },
                onBack: { manager.dispatch(.back) }
            )

        case .storageChoice:
            OnboardingStorageChoiceScreen(
                errorMessage: manager.state.errorMessage,
                onRestoreFromCoveBackup: onOpenCloudRestore,
                onSelectStorage: { selection in
                    manager.dispatch(.selectStorage(selection: selection))
                },
                onBack: { manager.dispatch(.back) }
            )

        case .softwareChoice:
            OnboardingSoftwareChoiceScreen(
                errorMessage: manager.state.errorMessage,
                onRestoreFromCoveBackup: onOpenCloudRestore,
                onSelectSoftwareAction: { selection in
                    manager.dispatch(.selectSoftwareAction(selection: selection))
                },
                onBack: { manager.dispatch(.back) }
            )

        case .creatingWallet:
            OnboardingCreatingWalletView {
                manager.dispatch(.continueWalletCreation)
            }

        case .backupWallet:
            OnboardingBackupWalletView(
                branch: manager.state.branch,
                secretWordsSaved: manager.state.secretWordsSaved,
                cloudBackupEnabled: manager.state.cloudBackupEnabled,
                wordCount: manager.state.createdWords.count,
                onShowWords: { manager.dispatch(.showSecretWords) },
                onEnableCloudBackup: { manager.dispatch(.openCloudBackup) },
                onContinue: { manager.dispatch(.continueFromBackup) }
            )

        case .cloudBackup:
            OnboardingCloudBackupStepView(
                branch: manager.state.branch,
                onEnabled: { manager.dispatch(.cloudBackupEnabled) },
                onSkip: { manager.dispatch(.skipCloudBackup) }
            )

        case .secretWords:
            OnboardingSecretWordsView(
                words: manager.state.createdWords,
                onBack: { manager.dispatch(.back) },
                onSaved: { manager.dispatch(.secretWordsSaved) }
            )

        case .exchangeFunding:
            OnboardingExchangeFundingView(
                walletId: manager.rust.currentWalletId(),
                onContinue: { manager.dispatch(.continueFromExchangeFunding) }
            )

        case .hardwareImport:
            OnboardingHardwareImportFlowView(
                onImported: { walletId in
                    manager.dispatch(.hardwareImportCompleted(walletId: walletId))
                },
                onBack: { manager.dispatch(.back) }
            )

        case .softwareImport:
            OnboardingSoftwareImportFlowView(
                onImported: { walletId in
                    manager.dispatch(.softwareImportCompleted(walletId: walletId))
                },
                onBack: { manager.dispatch(.back) }
            )
        }
    }
}
