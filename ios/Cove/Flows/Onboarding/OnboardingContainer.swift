import SwiftUI

struct OnboardingContainer: View {
    let manager: OnboardingManager
    let auth: AuthManager
    let onComplete: () -> Void

    var body: some View {
        CloudBackupPresentationHost(
            app: manager.app,
            auth: auth,
            isCoverPresented: false,
            presentationPolicy: .onboarding
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
            CloudCheckContent {
                manager.dispatch(.continueWithoutCloudRestore)
            }

        case .restoreOffer:
            CloudRestoreOfferView(
                onRestore: {
                    manager.dispatch(.startRestore)
                },
                onSkip: {
                    manager.dispatch(.skipRestore)
                },
                warningMessage: restoreWarningMessage,
                errorMessage: manager.state.errorMessage,
                providerHint: manager.state.cloudRestoreProviderHint
            )

        case .restoreOffline:
            OnboardingRestoreOfflineScreen(
                onContinue: { manager.dispatch(.continueWithoutCloudRestore) },
                onBack: { manager.dispatch(.back) }
            )

        case .restoreUnavailable:
            OnboardingRestoreUnavailableScreen(
                onContinue: { manager.dispatch(.continueWithoutCloudRestore) },
                onCheckAgain: { manager.dispatch(.retryCloudCheck) }
            )

        case .restoring, .restoreComplete, .restoreFailed:
            DeviceRestoreView(
                restoreState: manager.state.restoreState,
                onDone: { manager.dispatch(.continueFromRestoreComplete) },
                onRetry: { manager.dispatch(.retryRestore) },
                onContinueWithoutBackup: { manager.dispatch(.skipRestore) }
            )

        case .welcome:
            OnboardingWelcomeScreen(
                errorMessage: manager.state.errorMessage,
                onRestoreFromCoveBackup: { manager.dispatch(.openCloudRestore) },
                onContinue: { manager.dispatch(.continueFromWelcome) }
            )

        case .bitcoinChoice:
            OnboardingBitcoinChoiceScreen(
                errorMessage: manager.state.errorMessage,
                onRestoreFromCoveBackup: { manager.dispatch(.openCloudRestore) },
                onNewHere: { manager.dispatch(.selectHasBitcoin(hasBitcoin: false)) },
                onHasBitcoin: { manager.dispatch(.selectHasBitcoin(hasBitcoin: true)) }
            )

        case .storageChoice:
            OnboardingStorageChoiceScreen(
                errorMessage: manager.state.errorMessage,
                onRestoreFromCoveBackup: { manager.dispatch(.openCloudRestore) },
                onSelectStorage: { selection in
                    manager.dispatch(.selectStorage(selection: selection))
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
                isCloudRestoreCheckPending: manager.state.cloudRestoreState == .checking,
                onEnable: { manager.dispatch(.beginCloudBackupEnable) },
                onEnabled: { manager.dispatch(.cloudBackupEnabled) },
                onSkip: { manager.dispatch(.skipCloudBackup) }
            )

        case .cloudBackupSuccess:
            OnboardingCloudBackupSuccessView {
                manager.dispatch(.continueFromCloudBackupSuccess)
            }

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
                cloudRestoreAlertVisible: cloudRestoreAlertVisibleBinding,
                onImported: { walletId in
                    manager.dispatch(.hardwareImportCompleted(walletId: walletId))
                },
                onRestoreFromCloudBackup: { manager.dispatch(.openCloudRestore) },
                onDismissCloudRestoreAlert: { manager.dispatch(.dismissCloudRestoreAlert) },
                onBack: { manager.dispatch(.back) }
            )

        case .softwareImport:
            OnboardingSoftwareImportFlowView(
                errorMessage: manager.state.errorMessage,
                cloudRestoreAlertVisible: cloudRestoreAlertVisibleBinding,
                onImported: { walletId in
                    manager.dispatch(.softwareImportCompleted(walletId: walletId))
                },
                onCreateWallet: { manager.dispatch(.createSoftwareWallet) },
                onRestoreFromCloudBackup: { manager.dispatch(.openCloudRestore) },
                onDismissCloudRestoreAlert: { manager.dispatch(.dismissCloudRestoreAlert) },
                onBack: { manager.dispatch(.back) }
            )
        }
    }

    private var cloudRestoreAlertVisibleBinding: Binding<Bool> {
        Binding(
            get: { manager.state.cloudRestoreAlertVisible },
            set: { isPresented in
                if !isPresented {
                    manager.dispatch(.dismissCloudRestoreAlert)
                }
            }
        )
    }
}
