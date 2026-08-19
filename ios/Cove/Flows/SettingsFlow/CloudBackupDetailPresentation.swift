import SwiftUI

enum CloudBackupDestructiveConfirmation: Equatable {
    case recreate
    case reinitialize
}

enum CloudBackupDetailDialog {
    case destructive(CloudBackupDestructiveConfirmation)
    case cloudOnlyWalletActions(CloudBackupWalletItem)
    case disableCloudBackup
    case recoverOtherBackups
}

enum CloudBackupDetailAlert {
    case cloudOnlyDeleteWallet(CloudBackupWalletItem)
    case cloudOnlyUnsupportedRestore(CloudBackupWalletItem)
    case disableUnavailable(String)
    case disableFinalConfirmation
    case otherBackupsRecoveryResult(OtherBackupsRecoveryResult)
    case otherBackupsDeleteConfirmation
    case otherBackupsFinalDeleteConfirmation
}

enum CloudBackupDetailPresentation {
    case dialog(CloudBackupDetailDialog)
    case alert(CloudBackupDetailAlert)
}

extension View {
    func cloudBackupDetailPresentations(
        manager: CloudBackupManager,
        coordinator: PresentationTransitionCoordinator<CloudBackupDetailPresentation>
    ) -> some View {
        modifier(CloudBackupDetailPresentationModifier(
            manager: manager,
            coordinator: coordinator
        ))
    }
}

private struct CloudBackupDetailPresentationModifier: ViewModifier {
    let manager: CloudBackupManager
    let coordinator: PresentationTransitionCoordinator<CloudBackupDetailPresentation>

    private var dialog: CloudBackupDetailDialog? {
        guard case let .dialog(dialog) = coordinator.currentPresentation?.item else {
            return nil
        }

        return dialog
    }

    private var alert: CloudBackupDetailAlert? {
        guard case let .alert(alert) = coordinator.currentPresentation?.item else {
            return nil
        }

        return alert
    }

    private var dialogIsPresented: Binding<Bool> {
        coordinator.isPresented { presentation in
            if case .dialog = presentation { true } else { false }
        }
    }

    private var alertIsPresented: Binding<Bool> {
        coordinator.isPresented { presentation in
            if case .alert = presentation { true } else { false }
        }
    }

    func body(content: Content) -> some View {
        content
            .confirmationDialog(
                dialog.map(dialogTitle) ?? "Cloud Backup",
                isPresented: dialogIsPresented,
                titleVisibility: .visible,
                presenting: dialog,
                actions: dialogActions,
                message: dialogMessage
            )
            .alert(
                alert.map(alertTitle) ?? "Cloud Backup",
                isPresented: alertIsPresented,
                presenting: alert,
                actions: alertActions,
                message: alertMessage
            )
    }

    private func dialogTitle(_ dialog: CloudBackupDetailDialog) -> String {
        switch dialog {
        case let .destructive(confirmation):
            switch confirmation {
            case .recreate: "Recreate Backup Index"
            case .reinitialize: "Reinitialize Cloud Backup"
            }
        case let .cloudOnlyWalletActions(wallet):
            wallet.name
        case .disableCloudBackup:
            "Disable Cloud Backup?"
        case .recoverOtherBackups:
            "Recover wallets from another passkey?"
        }
    }

    @ViewBuilder
    private func dialogActions(_ dialog: CloudBackupDetailDialog) -> some View {
        switch dialog {
        case let .destructive(confirmation):
            Button(destructiveActionTitle(confirmation), role: .destructive) {
                performDestructiveAction(confirmation)
            }
            .disabled(!manager.isDetailInventoryComplete)

            Button("Cancel", role: .cancel) {}

        case let .cloudOnlyWalletActions(wallet):
            Button("Restore to This Device") {
                restoreCloudOnlyWallet(wallet)
            }
            .disabled(!manager.isDetailInventoryComplete)

            Button("Delete from iCloud", role: .destructive) {
                requestCloudOnlyWalletDeletion(wallet)
            }
            .disabled(!manager.isDetailInventoryComplete)

            Button("Cancel", role: .cancel) {}

        case .disableCloudBackup:
            Button("Continue", role: .destructive, action: presentFinalDisableConfirmation)
                .disabled(!manager.isDetailInventoryComplete)

            Button("Cancel", role: .cancel) {}

        case .recoverOtherBackups:
            Button("Try Passkey", action: recoverOtherBackups)
                .disabled(!manager.isDetailInventoryComplete)

            Button("Cancel", role: .cancel) {}
        }
    }

    @ViewBuilder
    private func dialogMessage(_ dialog: CloudBackupDetailDialog) -> some View {
        switch dialog {
        case let .destructive(confirmation):
            switch confirmation {
            case .recreate:
                Text(
                    "This will rebuild the backup index from wallets on this device. Wallets that only exist in the cloud backup will no longer be referenced."
                )
            case .reinitialize:
                Text(
                    "This will replace your entire cloud backup. Wallets that only exist in the current cloud backup will be lost."
                )
            }
        case .cloudOnlyWalletActions:
            EmptyView()
        case .disableCloudBackup:
            Text(
                "Disabling Cloud Backup will permanently delete your current Cove cloud backups from cloud storage."
            )
        case .recoverOtherBackups:
            Text(
                "This will use the selected passkey once to decrypt these other backups. Your current Cloud Backup passkey will not change."
            )
        }
    }

    private func alertTitle(_ alert: CloudBackupDetailAlert) -> String {
        switch alert {
        case let .cloudOnlyDeleteWallet(wallet):
            "Delete \(wallet.name)?"
        case let .cloudOnlyUnsupportedRestore(wallet):
            "Can't Restore \(wallet.name)"
        case .disableUnavailable:
            "Cloud Backup Can't Be Disabled Yet"
        case .disableFinalConfirmation:
            "Delete Cloud Backups?"
        case .otherBackupsRecoveryResult:
            "Wallets Recovered"
        case .otherBackupsDeleteConfirmation:
            "Delete Other Cloud Backups?"
        case .otherBackupsFinalDeleteConfirmation:
            "This Cannot Be Undone"
        }
    }

    @ViewBuilder
    private func alertActions(_ alert: CloudBackupDetailAlert) -> some View {
        switch alert {
        case let .cloudOnlyDeleteWallet(wallet):
            Button("Delete Forever", role: .destructive) {
                deleteCloudOnlyWallet(wallet)
            }
            .disabled(!manager.isDetailInventoryComplete)

            Button("Cancel", role: .cancel) {}

        case .cloudOnlyUnsupportedRestore, .disableUnavailable:
            Button("OK", role: .cancel) {}

        case .disableFinalConfirmation:
            Button("Delete Cloud Backups and Disable", role: .destructive, action: disableCloudBackup)
                .disabled(!manager.isDetailInventoryComplete)

            Button("Cancel", role: .cancel) {}

        case .otherBackupsRecoveryResult:
            Button("Verify Current Passkey", action: verifyCurrentPasskey)
            Button("Done", role: .cancel) {}

        case .otherBackupsDeleteConfirmation:
            Button("Continue", role: .destructive, action: presentFinalOtherBackupsDeleteConfirmation)
                .disabled(!manager.isDetailInventoryComplete)

            Button("Cancel", role: .cancel) {}

        case .otherBackupsFinalDeleteConfirmation:
            Button("Delete", role: .destructive, action: deleteOtherBackups)
                .disabled(!manager.isDetailInventoryComplete)

            Button("Cancel", role: .cancel) {}
        }
    }

    @ViewBuilder
    private func alertMessage(_ alert: CloudBackupDetailAlert) -> some View {
        switch alert {
        case .cloudOnlyDeleteWallet:
            Text("This wallet backup will be permanently removed from iCloud")
        case .cloudOnlyUnsupportedRestore:
            Text("This backup uses a newer version of Cove and can't be restored on this device yet")
        case let .disableUnavailable(message):
            Text(message)
        case .disableFinalConfirmation:
            Text(
                "Disabling Cloud Backup will permanently delete your current Cove cloud backups from cloud storage. Wallets already on this device will stay on this device, but they will no longer be backed up to cloud storage."
            )
        case let .otherBackupsRecoveryResult(result):
            Text(result.message)
        case .otherBackupsDeleteConfirmation:
            Text("This will permanently remove these other backups from iCloud.")
        case .otherBackupsFinalDeleteConfirmation:
            Text(
                "These backups cannot be recovered later, even if you find the passkey that currently protects them."
            )
        }
    }

    private func destructiveActionTitle(
        _ confirmation: CloudBackupDestructiveConfirmation
    ) -> String {
        switch confirmation {
        case .recreate: "Recreate"
        case .reinitialize: "Reinitialize"
        }
    }

    private func performDestructiveAction(_ confirmation: CloudBackupDestructiveConfirmation) {
        guard manager.isDetailInventoryComplete else { return }

        coordinator.dismissCurrentPresentation()

        switch confirmation {
        case .recreate:
            manager.dispatch(action: .recreateManifest)
        case .reinitialize:
            manager.dispatch(action: .reinitializeBackup)
        }
    }

    private func restoreCloudOnlyWallet(_ wallet: CloudBackupWalletItem) {
        guard manager.isDetailInventoryComplete else { return }

        if wallet.syncStatus == .unsupportedVersion {
            coordinator.transition(to: .alert(.cloudOnlyUnsupportedRestore(wallet)))
            return
        }

        coordinator.dismissCurrentPresentation()
        manager.dispatch(action: .restoreCloudWallet(wallet.recordId))
    }

    private func requestCloudOnlyWalletDeletion(_ wallet: CloudBackupWalletItem) {
        guard manager.isDetailInventoryComplete else { return }

        coordinator.transition(to: .alert(.cloudOnlyDeleteWallet(wallet)))
    }

    private func presentFinalDisableConfirmation() {
        guard manager.isDetailInventoryComplete else { return }

        coordinator.transition(to: .alert(.disableFinalConfirmation))
    }

    private func recoverOtherBackups() {
        guard manager.isDetailInventoryComplete else { return }

        coordinator.dismissCurrentPresentation()
        manager.dispatch(action: .recoverOtherBackups)
    }

    private func deleteCloudOnlyWallet(_ wallet: CloudBackupWalletItem) {
        guard manager.isDetailInventoryComplete else { return }

        coordinator.dismissCurrentPresentation()
        manager.dispatch(action: .deleteCloudWallet(wallet.recordId))
    }

    private func disableCloudBackup() {
        guard manager.isDetailInventoryComplete else { return }

        coordinator.dismissCurrentPresentation()
        manager.dispatch(action: .disableCloudBackup)
    }

    private func verifyCurrentPasskey() {
        coordinator.dismissCurrentPresentation()
        manager.startVerification(source: .cloudBackupDetail)
    }

    private func presentFinalOtherBackupsDeleteConfirmation() {
        guard manager.isDetailInventoryComplete else { return }

        coordinator.transition(to: .alert(.otherBackupsFinalDeleteConfirmation))
    }

    private func deleteOtherBackups() {
        guard manager.isDetailInventoryComplete else { return }

        coordinator.dismissCurrentPresentation()
        manager.dispatch(action: .deleteOtherBackups)
    }
}
