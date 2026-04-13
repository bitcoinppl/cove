import SwiftUI

struct OnboardingContainer: View {
    @State private var manager: OnboardingManager
    let onComplete: () -> Void

    init(manager: OnboardingManager, onComplete: @escaping () -> Void) {
        _manager = State(initialValue: manager)
        self.onComplete = onComplete
    }

    var body: some View {
        stepView(for: manager.state.step)
            .onChange(of: manager.isComplete) { _, complete in
                guard complete else { return }
                manager.app.reloadWallets()
                onComplete()
            }
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
                    manager.cloudCheckWarning = nil
                    manager.dispatch(.startRestore)
                },
                onSkip: {
                    manager.cloudCheckWarning = nil
                    manager.dispatch(.skipRestore)
                },
                warningMessage: manager.cloudCheckWarning,
                errorMessage: manager.cloudCheckWarning == nil ? manager.state.errorMessage : nil
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
                onNewHere: { manager.dispatch(.selectHasBitcoin(hasBitcoin: false)) },
                onHasBitcoin: { manager.dispatch(.selectHasBitcoin(hasBitcoin: true)) }
            )

        case .storageChoice:
            OnboardingStorageChoiceScreen(
                onExchange: {
                    manager.dispatch(.selectStorage(selection: .exchange))
                },
                onHardwareWallet: {
                    manager.dispatch(.selectStorage(selection: .hardwareWallet))
                },
                onSoftwareWallet: {
                    manager.dispatch(.selectStorage(selection: .softwareWallet))
                },
                onBack: { manager.dispatch(.back) }
            )

        case .softwareChoice:
            OnboardingSoftwareChoiceScreen(
                onCreateWallet: {
                    manager.dispatch(.selectSoftwareAction(selection: .createNewWallet))
                },
                onImportWallet: {
                    manager.dispatch(.selectSoftwareAction(selection: .importExistingWallet))
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
                onEnabled: { manager.dispatch(.cloudBackupEnabled) },
                onSkip: { manager.dispatch(.skipCloudBackup) }
            )

        case .secretWords:
            OnboardingSecretWordsView(
                words: manager.state.createdWords,
                onBack: { manager.dispatch(.back) },
                onSaved: { manager.dispatch(.secretWordsSaved) }
            )

        case .verifyWords:
            if let walletId = manager.rust.currentWalletId() {
                VerifyWordsContainer(
                    id: walletId,
                    onVerified: { manager.dispatch(.verifyWordsCompleted) }
                )
            } else {
                OnboardingErrorScreen(
                    title: "Unable to verify words",
                    message: "The wallet was created, but the verification state could not be loaded."
                )
            }

        case .exchangeFunding:
            OnboardingExchangeFundingView(
                walletId: manager.rust.currentWalletId(),
                onContinue: { manager.dispatch(.continueFromExchangeFunding) }
            )

        case .hardwareDeviceSelection:
            OnboardingHardwareDeviceSelectionScreen(
                selectedDevice: manager.state.hardwareDevice,
                onSelect: { device in
                    manager.dispatch(.selectHardwareDevice(device: device))
                },
                onBack: { manager.dispatch(.back) }
            )

        case .hardwareImport:
            OnboardingHardwareImportFlowView(
                device: manager.state.hardwareDevice,
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
                onBackupImported: {
                    manager.dispatch(.backupImportCompleted)
                },
                onBack: { manager.dispatch(.back) }
            )
        }
    }
}
