import SwiftUI

struct OnboardingBackupStepView: View {
    let manager: OnboardingManager
    let step: OnboardingStep

    var body: some View {
        switch step {
        case .backupWallet:
            OnboardingBackupWalletStep(manager: manager)

        case .cloudBackup:
            OnboardingCloudBackupStep(manager: manager)

        case .cloudBackupSuccess:
            OnboardingCloudBackupSuccessStep(manager: manager)

        case .secretWords:
            OnboardingSecretWordsStep(manager: manager)

        case .exchangeFunding:
            OnboardingExchangeFundingStep(manager: manager)

        case .terms, .cloudCheck, .restoreOffer, .restoreOffline, .restoreUnavailable,
             .restoring, .restoreComplete, .restoreFailed, .welcome, .bitcoinChoice,
             .storageChoice, .creatingWallet, .hardwareImport, .softwareImport:
            EmptyView()
        }
    }
}

private struct OnboardingBackupWalletStep: View {
    let manager: OnboardingManager

    var body: some View {
        OnboardingBackupWalletView(
            branch: manager.state.branch,
            secretWordsSaved: manager.state.secretWordsSaved,
            cloudBackupEnabled: manager.state.cloudBackupEnabled,
            wordCount: manager.state.createdWords.count,
            onShowWords: { manager.dispatch(.showSecretWords) },
            onEnableCloudBackup: { manager.dispatch(.openCloudBackup) },
            onContinue: { manager.dispatch(.continueFromBackup) }
        )
    }
}

private struct OnboardingCloudBackupStep: View {
    let manager: OnboardingManager

    var body: some View {
        OnboardingCloudBackupStepView(
            branch: manager.state.branch,
            onEnable: { manager.dispatch(.beginCloudBackupEnable) },
            onEnabled: { manager.dispatch(.cloudBackupEnabled) },
            onSkip: { manager.dispatch(.skipCloudBackup) }
        )
    }
}

private struct OnboardingCloudBackupSuccessStep: View {
    let manager: OnboardingManager

    var body: some View {
        OnboardingCloudBackupSuccessView {
            manager.dispatch(.continueFromCloudBackupSuccess)
        }
    }
}

private struct OnboardingSecretWordsStep: View {
    let manager: OnboardingManager

    var body: some View {
        OnboardingSecretWordsView(
            words: manager.state.createdWords,
            onBack: { manager.dispatch(.back) },
            onSaved: { manager.dispatch(.secretWordsSaved) }
        )
    }
}

private struct OnboardingExchangeFundingStep: View {
    let manager: OnboardingManager

    var body: some View {
        OnboardingExchangeFundingView(
            walletId: manager.rust.currentWalletId(),
            onContinue: { manager.dispatch(.continueFromExchangeFunding) }
        )
    }
}
