import SwiftUI

struct OnboardingWalletImportStepView: View {
    let manager: OnboardingManager
    let step: OnboardingStep

    private var cloudRestoreAlertVisible: Binding<Bool> {
        Binding(
            get: { manager.state.cloudRestoreAlertVisible },
            set: { isPresented in
                if !isPresented {
                    manager.dispatch(.dismissCloudRestoreAlert)
                }
            }
        )
    }

    var body: some View {
        switch step {
        case .hardwareImport:
            OnboardingHardwareImportStep(
                manager: manager,
                cloudRestoreAlertVisible: cloudRestoreAlertVisible
            )

        case .softwareImport:
            OnboardingSoftwareImportStep(
                manager: manager,
                cloudRestoreAlertVisible: cloudRestoreAlertVisible
            )

        case .terms, .cloudCheck, .restoreOffer, .restoreOffline, .restoreUnavailable,
             .restoring, .restoreComplete, .restoreFailed, .welcome, .bitcoinChoice,
             .storageChoice, .creatingWallet, .backupWallet, .cloudBackup,
             .cloudBackupSuccess, .secretWords, .exchangeFunding:
            EmptyView()
        }
    }
}

private struct OnboardingHardwareImportStep: View {
    let manager: OnboardingManager
    let cloudRestoreAlertVisible: Binding<Bool>

    var body: some View {
        OnboardingHardwareImportFlowView(
            cloudRestoreAlertVisible: cloudRestoreAlertVisible,
            onImported: { manager.dispatch(.hardwareImportCompleted(walletId: $0)) },
            onRestoreFromCloudBackup: { manager.dispatch(.openCloudRestore) },
            onDismissCloudRestoreAlert: { manager.dispatch(.dismissCloudRestoreAlert) },
            onBack: { manager.dispatch(.back) }
        )
    }
}

private struct OnboardingSoftwareImportStep: View {
    let manager: OnboardingManager
    let cloudRestoreAlertVisible: Binding<Bool>

    var body: some View {
        OnboardingSoftwareImportFlowView(
            errorMessage: manager.state.errorMessage,
            cloudRestoreAlertVisible: cloudRestoreAlertVisible,
            onImported: { manager.dispatch(.softwareImportCompleted(walletId: $0)) },
            onCreateWallet: { manager.dispatch(.createSoftwareWallet) },
            onRestoreFromCloudBackup: { manager.dispatch(.openCloudRestore) },
            onDismissCloudRestoreAlert: { manager.dispatch(.dismissCloudRestoreAlert) },
            onBack: { manager.dispatch(.back) }
        )
    }
}
