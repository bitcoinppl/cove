import SwiftUI

struct OnboardingRestoreStepView: View {
    let manager: OnboardingManager
    let step: OnboardingStep

    var body: some View {
        switch step {
        case .terms:
            OnboardingTermsStep(manager: manager)

        case .cloudCheck:
            OnboardingCloudCheckStep(manager: manager)

        case .restoreOffer:
            OnboardingRestoreOfferStep(manager: manager)

        case .restoreOffline:
            OnboardingRestoreOfflineStep(manager: manager)

        case .restoreUnavailable:
            OnboardingRestoreUnavailableStep(manager: manager)

        case .restoring, .restoreComplete, .restoreFailed:
            OnboardingDeviceRestoreStep(manager: manager)

        case .welcome, .bitcoinChoice, .storageChoice, .creatingWallet, .backupWallet,
             .cloudBackup, .cloudBackupSuccess, .secretWords, .exchangeFunding,
             .hardwareImport, .softwareImport:
            EmptyView()
        }
    }
}

private struct OnboardingTermsStep: View {
    let manager: OnboardingManager

    var body: some View {
        TermsAndConditionsView(errorMessage: manager.state.errorMessage) {
            manager.dispatch(.acceptTerms)
        }
    }
}

private struct OnboardingCloudCheckStep: View {
    let manager: OnboardingManager

    var body: some View {
        CloudCheckContent {
            manager.dispatch(.continueSetup)
        }
    }
}

private struct OnboardingRestoreOfferStep: View {
    let manager: OnboardingManager

    private var warningMessage: String? {
        guard manager.state.cloudRestoreState == .inconclusive else { return nil }

        return manager.state.cloudRestoreMessage
    }

    var body: some View {
        CloudRestoreOfferView(
            onRestore: { manager.dispatch(.startRestore) },
            onSkip: { manager.dispatch(.skipRestore) },
            warningMessage: warningMessage,
            errorMessage: manager.state.errorMessage,
            providerHint: manager.state.cloudRestoreProviderHint
        )
    }
}

private struct OnboardingRestoreOfflineStep: View {
    let manager: OnboardingManager

    var body: some View {
        OnboardingRestoreOfflineScreen(
            onContinue: { manager.dispatch(.continueWithoutCloudRestore) },
            onBack: { manager.dispatch(.back) }
        )
    }
}

private struct OnboardingRestoreUnavailableStep: View {
    let manager: OnboardingManager

    var body: some View {
        OnboardingRestoreUnavailableScreen(
            onCheckAgain: { manager.dispatch(.checkCloudRestoreAgain) },
            onContinue: { manager.dispatch(.continueSetup) },
            onBack: { manager.dispatch(.back) }
        )
    }
}

private struct OnboardingDeviceRestoreStep: View {
    let manager: OnboardingManager

    var body: some View {
        DeviceRestoreView(
            restoreState: manager.state.restoreState,
            onDone: { manager.dispatch(.continueFromRestoreComplete) },
            onRetry: { manager.dispatch(.retryRestore) },
            onContinueWithoutBackup: { manager.dispatch(.skipRestore) }
        )
    }
}
