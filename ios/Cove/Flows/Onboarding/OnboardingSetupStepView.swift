import SwiftUI

struct OnboardingSetupStepView: View {
    let manager: OnboardingManager
    let step: OnboardingStep

    var body: some View {
        switch step {
        case .welcome:
            OnboardingWelcomeStep(manager: manager)

        case .bitcoinChoice:
            OnboardingBitcoinChoiceStep(manager: manager)

        case .storageChoice:
            OnboardingStorageChoiceStep(manager: manager)

        case .creatingWallet:
            OnboardingCreatingWalletStep(manager: manager)

        case .terms, .cloudCheck, .restoreOffer, .restoreOffline, .restoreUnavailable,
             .restoring, .restoreComplete, .restoreFailed, .backupWallet, .cloudBackup,
             .cloudBackupSuccess, .secretWords, .exchangeFunding, .hardwareImport,
             .softwareImport:
            EmptyView()
        }
    }
}

private struct OnboardingWelcomeStep: View {
    let manager: OnboardingManager

    var body: some View {
        OnboardingWelcomeScreen(errorMessage: manager.state.errorMessage) {
            manager.dispatch(.continueFromWelcome)
        }
    }
}

private struct OnboardingBitcoinChoiceStep: View {
    let manager: OnboardingManager

    var body: some View {
        OnboardingBitcoinChoiceScreen(
            errorMessage: manager.state.errorMessage,
            onRestoreFromCoveBackup: { manager.dispatch(.openCloudRestore) },
            onNewHere: { manager.dispatch(.selectHasBitcoin(hasBitcoin: false)) },
            onHasBitcoin: { manager.dispatch(.selectHasBitcoin(hasBitcoin: true)) }
        )
    }
}

private struct OnboardingStorageChoiceStep: View {
    let manager: OnboardingManager

    var body: some View {
        OnboardingStorageChoiceScreen(
            errorMessage: manager.state.errorMessage,
            onRestoreFromCoveBackup: { manager.dispatch(.openCloudRestore) },
            onSelectStorage: { manager.dispatch(.selectStorage(selection: $0)) },
            onBack: { manager.dispatch(.back) }
        )
    }
}

private struct OnboardingCreatingWalletStep: View {
    let manager: OnboardingManager

    var body: some View {
        OnboardingCreatingWalletView {
            manager.dispatch(.continueWalletCreation)
        }
    }
}
