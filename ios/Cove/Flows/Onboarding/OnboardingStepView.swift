import SwiftUI

struct OnboardingStepView: View {
    let manager: OnboardingManager

    private var route: OnboardingStepRoute {
        OnboardingStepRoute(step: manager.state.step)
    }

    var body: some View {
        switch route {
        case let .restore(step):
            OnboardingRestoreStepView(manager: manager, step: step)

        case let .setup(step):
            OnboardingSetupStepView(manager: manager, step: step)

        case let .backup(step):
            OnboardingBackupStepView(manager: manager, step: step)

        case let .walletImport(step):
            OnboardingWalletImportStepView(manager: manager, step: step)
        }
    }
}

private enum OnboardingStepRoute {
    case restore(OnboardingStep)
    case setup(OnboardingStep)
    case backup(OnboardingStep)
    case walletImport(OnboardingStep)

    init(step: OnboardingStep) {
        switch step {
        case .terms, .cloudCheck, .restoreOffer, .restoreOffline, .restoreUnavailable,
             .restoring, .restoreComplete, .restoreFailed:
            self = .restore(step)

        case .welcome, .bitcoinChoice, .storageChoice, .creatingWallet:
            self = .setup(step)

        case .backupWallet, .cloudBackup, .cloudBackupSuccess, .secretWords, .exchangeFunding:
            self = .backup(step)

        case .hardwareImport, .softwareImport:
            self = .walletImport(step)
        }
    }
}
