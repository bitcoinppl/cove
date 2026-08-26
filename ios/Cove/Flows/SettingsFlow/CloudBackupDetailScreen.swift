import SwiftUI

enum CloudBackupDetailProgressPresentation: Equatable {
    case none
    case inventoryInline
    case verificationCard
    case verificationInline
}

func cloudBackupDetailProgressPresentation(
    verificationState: CloudBackupVerificationState?,
    isInventoryChecking: Bool,
    hasRetainedDetail: Bool,
    hasVisibleWalletRows: Bool
) -> CloudBackupDetailProgressPresentation {
    if case .running = verificationState {
        return hasVisibleWalletRows ? .verificationInline : .verificationCard
    }

    if isInventoryChecking, hasRetainedDetail {
        return .inventoryInline
    }

    return .none
}

func cloudBackupHasVisibleWalletRows(
    detail: CloudBackupDetail?,
    cloudOnly: CloudOnlyState
) -> Bool {
    guard let detail else { return false }

    if !detail.upToDate.isEmpty || !detail.needsSync.isEmpty {
        return true
    }

    guard case let .loaded(wallets) = cloudOnly else { return false }
    return !wallets.isEmpty
}

struct CloudBackupDetailScreen: View {
    @Environment(\.dismiss) private var dismiss
    @Environment(CloudBackupPresentationCoordinator.self)
    private var cloudBackupPresentationCoordinator
    @State private var manager = CloudBackupManager.shared
    @State private var presentationCoordinator =
        PresentationTransitionCoordinator<CloudBackupDetailPresentation>()

    private var hasVerificationResult: Bool {
        switch manager.verificationState {
        case .verified, .needsAttention, .awaitingUploadConfirmation, .cancelled, .failed: true
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
        manager.detail == nil && !hasVerificationResult && !isCancelled
    }

    private var progressPresentation: CloudBackupDetailProgressPresentation {
        cloudBackupDetailProgressPresentation(
            verificationState: manager.verificationState,
            isInventoryChecking: manager.isDetailInventoryChecking,
            hasRetainedDetail: manager.detail != nil,
            hasVisibleWalletRows: cloudBackupHasVisibleWalletRows(
                detail: manager.detail,
                cloudOnly: manager.cloudOnly
            )
        )
    }

    private var hasCloudBackupPresentationBlocker: Bool {
        presentationCoordinator.hasPresentationActivity
    }

    var body: some View {
        CloudBackupDetailForm(
            manager: manager,
            isCancelled: isCancelled,
            isPasskeyMissing: isPasskeyMissing,
            isUnsupportedPasskeyProvider: isUnsupportedPasskeyProvider,
            shouldShowLoadingState: shouldShowLoadingState,
            progressPresentation: progressPresentation,
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
