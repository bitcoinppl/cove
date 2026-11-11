import SwiftUI

/// Settings > Cloud Backup
///
/// Draws the body and sections Rust projects, in Rust's order, and gates every control on the
/// screen's actions. The Turn Off dialogs are a function of the review Rust holds
struct CloudBackupDetailScreen: View {
    @Environment(\.dismiss) private var dismiss
    @Environment(\.scenePhase) private var scenePhase
    @Environment(CloudBackupPresentationCoordinator.self)
    private var cloudBackupPresentationCoordinator
    @State private var manager = CloudBackupManager.shared
    @State private var presentationCoordinator =
        PresentationTransitionCoordinator<CloudBackupDetailPresentation>()

    private var screen: CloudBackupScreen {
        manager.screen
    }

    private var disable: CloudBackupDisableScreen {
        screen.configured?.disable ?? .idle
    }

    private var otherBackupsOperation: OtherBackupsOperation? {
        for case let .otherBackups(section) in screen.configured?.sections ?? [] {
            return section.operation
        }

        return nil
    }

    private var outcomes: CloudBackupOutcomeContext {
        CloudBackupOutcomeContext(lastOutcome: screen.lastOutcome) {
            manager.dispatch(action: .dismissNotice(.lastOutcome))
        }
    }

    var body: some View {
        Form {
            CloudBackupDetailBody(
                screen: screen,
                manager: manager,
                presentationCoordinator: presentationCoordinator
            )
        }
        .environment(\.cloudBackupOutcomes, outcomes)
        .cloudBackupDetailPresentations(
            screen: screen,
            manager: manager,
            coordinator: presentationCoordinator
        )
        .navigationTitle("Cloud Backup")
        .navigationBarTitleDisplayMode(.inline)
        .presentationTransitionHost(presentationCoordinator)
        .task(enterDetail)
        .onDisappear(perform: leaveDetail)
        .onChange(of: presentationCoordinator.hasPresentationActivity, initial: true) { _, active in
            cloudBackupPresentationCoordinator.setBlocker(.cloudBackupDetailDialog, active: active)
        }
        .onChange(of: otherBackupsOperation, handleOtherBackupsOperationChange)
        .onChange(of: disable, initial: true) { _, disable in
            presentationCoordinator.syncTurnOffDialog(CloudBackupTurnOffDialogDemand(disable))
        }
        .onChange(of: scenePhase) { _, phase in
            // the app lock dismisses presented dialogs; a review Rust still holds comes back on unlock
            guard phase == .active else { return }
            presentationCoordinator.syncTurnOffDialog(CloudBackupTurnOffDialogDemand(disable))
        }
        .onChange(of: screen.body) { _, body in
            if case .disabled = body { dismiss() }
        }
    }

    private func enterDetail() async {
        manager.dispatch(action: .enterDetail)
    }

    private func leaveDetail() {
        // leaving the screen is a way out of the Turn Off review, like its Cancel button
        if case .reviewing = disable {
            manager.dispatch(action: .cancelDisable)
        }

        manager.dispatch(action: .leaveDetail)
        presentationCoordinator.discardAll()
        cloudBackupPresentationCoordinator.setBlocker(.cloudBackupDetailDialog, active: false)
    }

    private func handleOtherBackupsOperationChange(
        _: OtherBackupsOperation?,
        _ operation: OtherBackupsOperation?
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
}
