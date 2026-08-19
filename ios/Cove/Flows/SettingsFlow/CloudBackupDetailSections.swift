import SwiftUI

struct DetailFormContent: View {
    let detail: CloudBackupDetail
    let syncHealth: CloudSyncHealth
    let manager: CloudBackupManager
    let presentationCoordinator: PresentationTransitionCoordinator<CloudBackupDetailPresentation>

    private var showCloudOnlySection: Bool {
        switch manager.cloudOnly {
        case .notFetched: detail.cloudOnlyCount > 0
        case .loading: true
        case let .loaded(wallets): !wallets.isEmpty
        case .failed: true
        }
    }

    var body: some View {
        HeaderSection(lastSync: detail.lastSync, syncHealth: syncHealth)
        if !wallets.isEmpty {
            WalletSections(wallets: wallets)
        }
        if showCloudOnlySection {
            CloudOnlySection(
                manager: manager,
                presentationCoordinator: presentationCoordinator
            )
        }
        switch detail.otherBackups {
        case let .loaded(summary):
            if summary.namespaceCount > 0 {
                OtherBackupsSection(
                    summary: summary,
                    manager: manager,
                    presentationCoordinator: presentationCoordinator
                )
            }
        case let .loadFailed(error):
            OtherBackupsLoadFailedSection(error: error)
        }
    }

    private var wallets: [CloudBackupWalletItem] {
        detail.upToDate + detail.needsSync
    }
}

struct MissingPasskeyContent: View {
    let manager: CloudBackupManager

    private var isRepairing: Bool {
        if case .running = manager.passkeyRepairState { return true }
        return false
    }

    private var repairError: String? {
        if case let .failed(error) = manager.passkeyRepairState {
            return error
        }
        return nil
    }

    var body: some View {
        Section {
            VStack(spacing: 12) {
                Image(systemName: "exclamationmark.icloud.fill")
                    .font(.system(size: 36))
                    .foregroundStyle(Color.statusError)

                Text("Cloud Backup Passkey Missing")
                    .font(.headline)
                    .foregroundStyle(Color.statusError)

                Text(
                    "Your cloud backup is not accessible until you use an existing passkey or add a new one. Without it, your backups can't be restored."
                )
                .font(.subheadline)
                .foregroundStyle(Color.statusError.opacity(0.85))
                .multilineTextAlignment(.center)
            }
            .frame(maxWidth: .infinity)
            .padding(.vertical, 12)
        }

        Section {
            Button {
                manager.dispatch(action: .repairPasskeyNoDiscovery)
            } label: {
                if isRepairing {
                    HStack {
                        ProgressView()
                            .padding(.trailing, 4)
                        Text("Opening Passkey Options...")
                    }
                } else {
                    Label("Add Passkey", systemImage: "person.badge.key")
                }
            }
            .disabled(isRepairing)

            Text("Use an existing passkey or add a new one to make your cloud backup accessible again")
                .font(.caption)
                .foregroundStyle(.secondary)
        }

        if let repairError {
            Section {
                Label(repairError, systemImage: "exclamationmark.triangle.fill")
                    .foregroundStyle(Color.statusError)
                    .font(.caption)
            }
        }
    }
}

struct DisableCloudBackupSection: View {
    let manager: CloudBackupManager
    let detail: CloudBackupDetail?
    let presentationCoordinator: PresentationTransitionCoordinator<CloudBackupDetailPresentation>

    private var unavailableMessage: String? {
        if manager.isDisablingCloudBackup {
            return "Cove is already disabling Cloud Backup."
        }

        if manager.isPerformingDestructiveAction, manager.disableFailure == nil {
            return "Cove is waiting for the current Cloud Backup operation to finish."
        }

        if case .operating = manager.cloudOnlyOperation {
            return "Cove is waiting for the current cloud-only wallet operation to finish."
        }

        switch manager.otherBackupsOperation {
        case .recovering, .deleting:
            return "Cove is waiting for the current other-backup operation to finish."
        default:
            break
        }

        if let detail {
            if detail.cloudOnlyCount > 0 {
                return "Restore or delete wallets that are only in Cloud Backup before disabling."
            }

            if case let .loaded(summary) = detail.otherBackups, summary.namespaceCount > 0 {
                return "Recover or delete other Cloud Backups before disabling."
            }
        }

        return nil
    }

    var body: some View {
        DisableCloudBackupControls(
            manager: manager,
            unavailableMessage: unavailableMessage,
            presentationCoordinator: presentationCoordinator
        )
    }
}

private struct DisableCloudBackupControls: View {
    let manager: CloudBackupManager
    let unavailableMessage: String?
    let presentationCoordinator: PresentationTransitionCoordinator<CloudBackupDetailPresentation>

    var body: some View {
        Section {
            if manager.isDisablingCloudBackup {
                DisableCloudBackupProgress()
            }

            if let failure = manager.disableFailure {
                DisableCloudBackupFailureControls(
                    manager: manager,
                    message: failure.message,
                    canKeepEnabled: failure.canKeepEnabled
                )
            }

            DisableCloudBackupRequestButton(
                manager: manager,
                unavailableMessage: unavailableMessage,
                presentationCoordinator: presentationCoordinator
            )
        }
    }
}

private struct DisableCloudBackupRequestButton: View {
    let manager: CloudBackupManager
    let unavailableMessage: String?
    let presentationCoordinator: PresentationTransitionCoordinator<CloudBackupDetailPresentation>

    var body: some View {
        Button(role: .destructive, action: requestDisable) {
            Text("Disable Cloud Backup")
                .font(.footnote)
        }
        .disabled(manager.isDisablingCloudBackup || !manager.isDetailInventoryComplete)
        .alignmentGuide(.listRowSeparatorLeading) { _ in 0 }
    }

    private func requestDisable() {
        guard manager.isDetailInventoryComplete else { return }

        if let unavailableMessage {
            presentationCoordinator.present(.alert(.disableUnavailable(unavailableMessage)))
        } else {
            presentationCoordinator.present(.dialog(.disableCloudBackup))
        }
    }
}

private struct DisableCloudBackupProgress: View {
    var body: some View {
        HStack {
            ProgressView()
                .padding(.trailing, 8)
            Text("Deleting cloud backups...")
                .font(.footnote)
        }
    }
}

private struct DisableCloudBackupFailureControls: View {
    let manager: CloudBackupManager
    let message: String
    let canKeepEnabled: Bool

    var body: some View {
        Text(message)
            .font(.caption)
            .foregroundStyle(Color.statusError)

        Button(action: retryDisable) {
            Label("Try Again", systemImage: "arrow.clockwise")
        }
        .disabled(!manager.isDetailInventoryComplete)

        if canKeepEnabled {
            Button(action: keepEnabled) {
                Label("Keep Cloud Backup Enabled", systemImage: "icloud")
            }
        }
    }

    private func retryDisable() {
        guard manager.isDetailInventoryComplete else { return }

        manager.dispatch(action: .disableCloudBackup)
    }

    private func keepEnabled() {
        manager.dispatch(action: .keepCloudBackupEnabled)
    }
}
