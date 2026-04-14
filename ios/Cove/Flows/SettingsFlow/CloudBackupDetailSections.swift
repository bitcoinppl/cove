import SwiftUI

@_exported import CoveCore

private extension CloudOnlyOperation {
    var operatingRecordId: String? {
        if case let .operating(recordId) = self { return recordId }
        return nil
    }
}

struct DetailFormContent: View {
    let detail: CloudBackupDetail
    let syncHealth: CloudSyncHealth
    let manager: CloudBackupManager

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
        if !detail.upToDate.isEmpty {
            WalletSections(wallets: detail.upToDate)
        }
        if !detail.needsSync.isEmpty {
            WalletSections(wallets: detail.needsSync)
        }
        if showCloudOnlySection {
            CloudOnlySection(manager: manager)
        }
    }
}

struct MissingPasskeyContent: View {
    let manager: CloudBackupManager

    private var isRepairing: Bool {
        if case .recovering(.repairPasskey) = manager.recovery { return true }
        return false
    }

    private var repairError: String? {
        if case let .failed(action: .repairPasskey, error: error) = manager.recovery {
            return error
        }
        return nil
    }

    var body: some View {
        Section {
            VStack(spacing: 12) {
                Image(systemName: "exclamationmark.icloud.fill")
                    .font(.system(size: 36))
                    .foregroundStyle(.red)

                Text("Cloud Backup Passkey Missing")
                    .font(.headline)
                    .foregroundStyle(.red)

                Text(
                    "Your cloud backup is not accessible until you use an existing passkey or add a new one. Without it, your backups can't be restored."
                )
                .font(.subheadline)
                .foregroundStyle(.red.opacity(0.85))
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
                    .foregroundStyle(.red)
                    .font(.caption)
            }
        }
    }
}

struct HeaderSection: View {
    let lastSync: UInt64?
    let syncHealth: CloudSyncHealth

    var body: some View {
        Section {
            VStack(spacing: 8) {
                headerIcon
                    .font(.largeTitle)

                Text("Cloud Backup Active")
                    .fontWeight(.semibold)

                if let lastSync {
                    Text("Last synced \(formatDate(lastSync))")
                        .font(.caption)
                        .foregroundStyle(.secondary)

                    syncHealthLabel
                }
            }
            .frame(maxWidth: .infinity)
            .padding(.vertical, 8)
        }
    }

    @ViewBuilder
    private var headerIcon: some View {
        switch syncHealth {
        case .allUploaded, .noFiles:
            Image(systemName: "checkmark.icloud.fill")
                .foregroundColor(.green)
        case .uploading:
            Image(systemName: "arrow.clockwise.icloud.fill")
                .foregroundColor(.blue)
        case .failed:
            Image(systemName: "exclamationmark.icloud.fill")
                .foregroundColor(.red)
        case .unavailable:
            Image(systemName: "checkmark.icloud.fill")
                .foregroundColor(.green)
        }
    }

    @ViewBuilder
    private var syncHealthLabel: some View {
        switch syncHealth {
        case .allUploaded:
            Label("All files synced to iCloud", systemImage: "checkmark.circle.fill")
                .font(.caption)
                .foregroundStyle(.green)
        case .uploading:
            HStack(spacing: 4) {
                ProgressView()
                    .controlSize(.mini)
                Text("Syncing to iCloud...")
            }
            .font(.caption)
            .foregroundStyle(.secondary)
        case let .failed(message):
            Label("Sync error: \(message)", systemImage: "exclamationmark.triangle.fill")
                .font(.caption)
                .foregroundStyle(.red)
        case .noFiles, .unavailable:
            EmptyView()
        }
    }

    private func formatDate(_ timestamp: UInt64) -> String {
        let date = Date(timeIntervalSince1970: TimeInterval(timestamp))
        return date.formatted(date: .abbreviated, time: .shortened)
    }
}

struct CloudOnlySection: View {
    let manager: CloudBackupManager
    @State private var selectedWallet: CloudBackupWalletItem?
    @State private var walletToDelete: CloudBackupWalletItem?
    @State private var unsupportedRestoreWallet: CloudBackupWalletItem?

    private var isOperating: Bool {
        manager.cloudOnlyOperation.operatingRecordId != nil
    }

    var body: some View {
        Section(header: Text("Not on This Device")) {
            CloudOnlySectionContent(
                manager: manager,
                isOperating: isOperating,
                onSelectWallet: { selectedWallet = $0 }
            )
        }
        .modifier(
            CloudOnlyActionDialogs(
                manager: manager,
                selectedWallet: $selectedWallet,
                walletToDelete: $walletToDelete,
                unsupportedRestoreWallet: $unsupportedRestoreWallet
            )
        )
    }
}

private struct CloudOnlySectionContent: View {
    let manager: CloudBackupManager
    let isOperating: Bool
    let onSelectWallet: (CloudBackupWalletItem) -> Void

    var body: some View {
        switch manager.cloudOnly {
        case .notFetched, .loading:
            HStack {
                ProgressView()
                    .padding(.trailing, 8)
                Text("Loading...")
            }
            .foregroundStyle(.secondary)
            .task {
                manager.dispatch(action: .fetchCloudOnly)
            }

        case let .loaded(wallets):
            CloudOnlyWalletRows(
                wallets: wallets,
                operatingRecordId: manager.cloudOnlyOperation.operatingRecordId,
                isOperating: isOperating,
                onSelectWallet: onSelectWallet
            )

            if case let .failed(error) = manager.cloudOnlyOperation {
                Text(error)
                    .font(.caption)
                    .foregroundStyle(.red)
            } else if case let .warning(message: message, error: _) = manager.cloudOnlyOperation {
                Text(message)
                    .font(.caption)
                    .foregroundStyle(.orange)
            }

        case let .failed(error):
            Text(error)
                .font(.caption)
                .foregroundStyle(.red)
        }
    }
}

private struct CloudOnlyWalletRows: View {
    let wallets: [CloudBackupWalletItem]
    let operatingRecordId: String?
    let isOperating: Bool
    let onSelectWallet: (CloudBackupWalletItem) -> Void

    var body: some View {
        ForEach(wallets, id: \.recordId) { item in
            Button {
                onSelectWallet(item)
            } label: {
                HStack {
                    if operatingRecordId == item.recordId {
                        ProgressView()
                            .padding(.trailing, 8)
                    }
                    WalletItemRow(item: item)
                }
            }
            .foregroundStyle(.primary)
            .disabled(isOperating)
        }
    }
}

private struct CloudOnlyActionDialogs: ViewModifier {
    let manager: CloudBackupManager
    @Binding var selectedWallet: CloudBackupWalletItem?
    @Binding var walletToDelete: CloudBackupWalletItem?
    @Binding var unsupportedRestoreWallet: CloudBackupWalletItem?

    func body(content: Content) -> some View {
        content
            .confirmationDialog(
                selectedWallet?.name ?? "Wallet",
                isPresented: Binding(
                    get: { selectedWallet != nil },
                    set: { if !$0 { selectedWallet = nil } }
                ),
                titleVisibility: .visible
            ) {
                if let item = selectedWallet {
                    Button("Restore to This Device") {
                        if item.syncStatus == .unsupportedVersion {
                            unsupportedRestoreWallet = item
                            return
                        }

                        manager.dispatch(action: .restoreCloudWallet(recordId: item.recordId))
                    }
                    Button("Delete from iCloud", role: .destructive) {
                        walletToDelete = item
                    }
                }
                Button("Cancel", role: .cancel) {}
            }
            .alert(
                "Delete \(walletToDelete?.name ?? "wallet")?",
                isPresented: Binding(
                    get: { walletToDelete != nil },
                    set: { if !$0 { walletToDelete = nil } }
                )
            ) {
                if let item = walletToDelete {
                    Button("Delete", role: .destructive) {
                        manager.dispatch(action: .deleteCloudWallet(recordId: item.recordId))
                    }
                }
                Button("Cancel", role: .cancel) {}
            } message: {
                Text("This wallet backup will be permanently removed from iCloud")
            }
            .alert(
                "Can't Restore \(unsupportedRestoreWallet?.name ?? "Wallet")",
                isPresented: Binding(
                    get: { unsupportedRestoreWallet != nil },
                    set: { if !$0 { unsupportedRestoreWallet = nil } }
                )
            ) {
                Button("OK", role: .cancel) {}
            } message: {
                Text(
                    "This backup uses a newer version of Cove and can't be restored on this device yet"
                )
            }
    }
}

struct WalletSections: View {
    let wallets: [CloudBackupWalletItem]

    private let groupedWallets: GroupedWalletSections

    init(wallets: [CloudBackupWalletItem]) {
        self.wallets = wallets
        groupedWallets = GroupedWalletSections(wallets: wallets)
    }

    var body: some View {
        ForEach(groupedWallets.sections) { group in
            Section(header: sectionHeader(for: group.key)) {
                ForEach(group.items, id: \.recordId) { item in
                    WalletItemRow(item: item)
                }
            }
        }
    }

    private func sectionHeader(for key: GroupKey) -> some View {
        Text(key.title)
    }
}

private struct GroupedWalletSections {
    struct Section: Identifiable {
        let key: GroupKey
        let items: [CloudBackupWalletItem]

        var id: GroupKey {
            key
        }
    }

    let sections: [Section]

    init(wallets: [CloudBackupWalletItem]) {
        sections = Dictionary(grouping: wallets) {
            GroupKey(network: $0.network, walletMode: $0.walletMode)
        }
        .map { key, items in
            Section(key: key, items: items)
        }
        .sorted { $0.key < $1.key }
    }
}

struct WalletItemRow: View {
    let item: CloudBackupWalletItem

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            HStack {
                Text(item.name)
                    .fontWeight(.medium)
                Spacer()
                StatusBadge(status: item.syncStatus)
            }

            HStack(spacing: 12) {
                if let network = item.network {
                    IconLabel("globe", network.displayName())
                }
                if let walletType = item.walletType {
                    IconLabel("wallet.bifold", walletType.displayName())
                }
                if let fingerprint = item.fingerprint {
                    IconLabel("touchid", fingerprint)
                }
            }
            .font(.caption)
            .foregroundStyle(.secondary)

            HStack(spacing: 12) {
                if let labelCount = item.labelCount {
                    IconLabel("tag", "\(labelCount) labels")
                }
                if let backupUpdatedAt = item.backupUpdatedAt {
                    IconLabel("clock", formatDate(backupUpdatedAt))
                }
            }
            .font(.caption)
            .foregroundStyle(.secondary)
        }
        .padding(.vertical, 2)
    }

    private func formatDate(_ timestamp: UInt64) -> String {
        let date = Date(timeIntervalSince1970: TimeInterval(timestamp))
        return date.formatted(date: .abbreviated, time: .shortened)
    }
}

private struct StatusBadge: View {
    let status: CloudBackupWalletStatus

    private var label: String {
        switch status {
        case .dirty: "Dirty"
        case .uploading: "Uploading"
        case .uploadedPendingConfirmation: "Uploaded, confirming"
        case .confirmed: "Confirmed"
        case .failed: "Failed"
        case .deletedFromDevice: "Not on device"
        case .unsupportedVersion: "Unsupported"
        case .remoteStateUnknown: "Unknown"
        }
    }

    private var color: Color {
        switch status {
        case .dirty: .orange
        case .uploading, .uploadedPendingConfirmation: .blue
        case .confirmed: .green
        case .failed: .red
        case .deletedFromDevice, .unsupportedVersion: .orange
        case .remoteStateUnknown: .secondary
        }
    }

    var body: some View {
        Text(label)
            .font(.caption)
            .fontWeight(.medium)
            .foregroundColor(color)
            .padding(.horizontal, 8)
            .padding(.vertical, 3)
            .background(color.opacity(0.15), in: Capsule())
    }
}

private struct GroupKey: Hashable, Comparable {
    let network: Network?
    let walletMode: WalletMode?

    var title: String {
        guard let network, let walletMode else {
            return "Unsupported"
        }

        return switch walletMode {
        case .decoy: "\(network.displayName()) · Decoy"
        default: network.displayName()
        }
    }

    static func < (lhs: GroupKey, rhs: GroupKey) -> Bool {
        if lhs.network == nil || lhs.walletMode == nil {
            return rhs.network != nil && rhs.walletMode != nil
        }
        if rhs.network == nil || rhs.walletMode == nil {
            return false
        }

        let lhsNetwork = lhs.network!
        let rhsNetwork = rhs.network!
        if lhsNetwork != rhsNetwork {
            return lhsNetwork.displayName() < rhsNetwork.displayName()
        }
        return lhs.walletMode == .main && rhs.walletMode != .main
    }
}
