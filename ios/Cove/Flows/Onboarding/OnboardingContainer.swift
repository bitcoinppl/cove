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

        return manager.state.cloudRestoreIssue?.localizedMessage
    }

    private var localizedErrorMessage: String? {
        guard manager.state.errorMessage != nil else { return nil }

        switch manager.state.step {
        case .terms:
            return String(localized: "Unable to complete onboarding. Please try again.")
        case .restoreOffer, .restoring, .restoreComplete, .restoreFailed:
            return String(localized: "Unable to restore from Cloud Backup. Please try again.")
        case .welcome, .bitcoinChoice, .storageChoice, .creatingWallet, .backupWallet, .cloudBackup,
             .cloudBackupSuccess, .secretWords, .exchangeFunding, .hardwareImport, .softwareImport,
             .cloudCheck, .restoreOffline, .restoreUnavailable:
            return String(localized: "Unable to continue setup. Please try again.")
        }
    }

    @ViewBuilder
    private func stepView(for step: OnboardingStep) -> some View {
        switch step {
        case .terms:
            TermsAndConditionsView(errorMessage: localizedErrorMessage) {
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
                errorMessage: localizedErrorMessage,
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
                onBack: { manager.dispatch(.back) }
            )

        case .restoring, .restoreComplete, .restoreFailed:
            DeviceRestoreView(
                restoreState: manager.state.restoreState,
                onDone: { manager.dispatch(.continueFromRestoreComplete) },
                onRetry: { manager.dispatch(.retryRestore) },
                onContinueWithoutBackup: { manager.dispatch(.skipRestore) }
            )

        case .welcome:
            OnboardingWelcomeScreen(errorMessage: localizedErrorMessage) {
                manager.dispatch(.continueFromWelcome)
            }

        case .bitcoinChoice:
            OnboardingBitcoinChoiceScreen(
                errorMessage: localizedErrorMessage,
                onNewHere: { manager.dispatch(.selectHasBitcoin(hasBitcoin: false)) },
                onHasBitcoin: { manager.dispatch(.selectHasBitcoin(hasBitcoin: true)) }
            )

        case .storageChoice:
            OnboardingStorageChoiceScreen(
                errorMessage: localizedErrorMessage,
                onRestoreFromCoveBackup: onOpenCloudRestore,
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
                errorMessage: localizedErrorMessage,
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
