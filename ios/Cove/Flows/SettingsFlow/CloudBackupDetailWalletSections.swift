import SwiftUI

struct WalletSections: View {
    let wallets: [CloudBackupWalletItem]

    private var groupedWallets: [GroupedWalletSections.Section] {
        GroupedWalletSections(wallets: wallets).sections
    }

    var body: some View {
        ForEach(groupedWallets) { group in
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
                    IconLabel("globe", network.displayName)
                }
                if let walletType = item.walletType {
                    IconLabel("wallet.bifold", walletType.localizedDisplayName)
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
        case .dirty: .statusWarning
        case .uploading, .uploadedPendingConfirmation: .statusInfo
        case .confirmed: .statusSuccess
        case .failed: .statusError
        case .deletedFromDevice, .unsupportedVersion: .statusWarning
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
        case .decoy: "\(network.displayName) · Decoy"
        default: network.displayName
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
            return lhsNetwork.displayName < rhsNetwork.displayName
        }
        return lhs.walletMode == .main && rhs.walletMode != .main
    }
}
