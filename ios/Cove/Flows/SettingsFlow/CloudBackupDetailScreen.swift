import SwiftUI

struct CloudBackupDetailScreen: View {
    @Environment(\.dismiss) private var dismiss
    @Environment(CloudBackupPresentationCoordinator.self)
    private var cloudBackupPresentationCoordinator
    @State private var manager = CloudBackupManager.shared
    @State private var presentationCoordinator =
        PresentationTransitionCoordinator<CloudBackupDetailPresentation>()

    private var isVerifying: Bool {
        if case .running = manager.verificationState { return true }
        return false
    }

    private var hasVerificationResult: Bool {
        switch manager.verificationState {
        case .verified, .awaitingUploadConfirmation, .cancelled, .failed: true
        default: false
        }
    }

    private var isCancelled: Bool {
        if case .cancelled = manager.verificationState {
            return true
        }
        return false
    }

    private var isPasskeyMissing: Bool {
        manager.isPasskeyMissing
    }

    private var isUnsupportedPasskeyProvider: Bool {
        manager.isUnsupportedPasskeyProvider
    }

    private var shouldShowLoadingState: Bool {
        manager.detail == nil && !isVerifying && !hasVerificationResult && !isCancelled
    }

    private var hasCloudBackupPresentationBlocker: Bool {
        presentationCoordinator.hasPresentationActivity
    }

    var body: some View {
        CloudBackupDetailForm(
            manager: manager,
            isVerifying: isVerifying,
            hasVerificationResult: hasVerificationResult,
            isCancelled: isCancelled,
            isPasskeyMissing: isPasskeyMissing,
            isUnsupportedPasskeyProvider: isUnsupportedPasskeyProvider,
            shouldShowLoadingState: shouldShowLoadingState,
            presentationCoordinator: presentationCoordinator,
            recreateConfirmationIsPresented: confirmationBinding(for: .recreate),
            reinitializeConfirmationIsPresented: confirmationBinding(for: .reinitialize)
        )
        .cloudBackupDetailPresentations(
            manager: manager,
            coordinator: presentationCoordinator
        )
        .navigationTitle("Cloud Backup")
        .navigationBarTitleDisplayMode(.inline)
        .presentationTransitionHost(presentationCoordinator)
        .task(enterDetail)
        .onDisappear(perform: closeDetail)
        .onChange(of: hasCloudBackupPresentationBlocker, initial: true) { _, active in
            cloudBackupPresentationCoordinator.setBlocker(.cloudBackupDetailDialog, active: active)
        }
        .onChange(of: manager.otherBackupsOperation, handleOtherBackupsOperationChange)
        .onChange(of: manager.isLifecycleDisabled) { _, isDisabled in
            if isDisabled {
                dismiss()
            }
        }
    }

    private func confirmationBinding(
        for confirmation: CloudBackupDestructiveConfirmation
    ) -> Binding<Bool> {
        let presentationBinding = presentationCoordinator.isPresented { presentation in
            guard case let .dialog(.destructive(current)) = presentation else {
                return false
            }

            return current == confirmation
        }

        return Binding(
            get: { presentationBinding.wrappedValue },
            set: { presented in
                if presented {
                    guard manager.isDetailInventoryComplete else { return }

                    presentationCoordinator.present(.dialog(.destructive(confirmation)))
                } else {
                    presentationBinding.wrappedValue = false
                }
            }
        )
    }

    private func enterDetail() async {
        manager.dispatch(action: .enterDetail)
    }

    private func handleOtherBackupsOperationChange(
        _: OtherBackupsOperation,
        _ operation: OtherBackupsOperation
    ) {
        guard case let .recovered(walletsRestored, walletsFailed, failedWalletErrors) = operation else {
            return
        }

        presentationCoordinator.present(
            .alert(
                .otherBackupsRecoveryResult(
                    OtherBackupsRecoveryResult(
                        walletsRestored: walletsRestored,
                        walletsFailed: walletsFailed,
                        failedWalletErrors: failedWalletErrors
                    )
                )
            )
        )
    }

    private func closeDetail() {
        presentationCoordinator.discardAll()
        manager.dispatch(action: .closeDetail)
        cloudBackupPresentationCoordinator.setBlocker(.cloudBackupDetailDialog, active: false)
    }
}
