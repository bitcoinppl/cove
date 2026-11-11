import SwiftUI

struct WalletSections: View {
    let wallets: [CloudBackupWalletItem]

    private var groupedWallets: [GroupedWalletSections.Section] {
        GroupedWalletSections(wallets: wallets).sections
    }

    var body: some View {
        ForEach(groupedWallets) { group in
            Section(header: WalletSectionHeader(key: group.key)) {
                ForEach(group.items, id: \.recordId) { item in
                    WalletItemRow(item: item)
                        .uiTestTruncationReport("cloudBackup.wallet.\(item.recordId).truncation")
                }
            }
        }
    }
}

private struct WalletSectionHeader: View {
    let key: GroupKey

    var body: some View {
        Text(key.title)
            .uiTestTruncationProbe()
    }
}

struct WalletItemRow: View {
    @Environment(\.dynamicTypeSize) private var dynamicTypeSize

    let item: CloudBackupWalletItem
    var accessibilityAction: String?

    init(item: CloudBackupWalletItem, accessibilityAction: String? = nil) {
        self.item = item
        self.accessibilityAction = accessibilityAction
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            // at accessibility sizes the badge beside the name broke words such as Confirmed
            // across lines, so it goes under the name instead
            if dynamicTypeSize.isAccessibilitySize {
                name
                StatusBadge(status: item.syncStatus)
            } else {
                HStack {
                    name
                    Spacer()
                    StatusBadge(status: item.syncStatus)
                }
            }

            WalletItemMetadata(item: item)

            if let failure = item.restoreFailure {
                ProbedLabel(failure.message, systemImage: "exclamationmark.circle")
                    .font(.caption)
                    .foregroundStyle(Color.statusError)
                    .fixedSize(horizontal: false, vertical: true)
            }
        }
        .padding(.vertical, 2)
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(cloudBackupWalletAccessibilityLabel(
            item: item,
            action: accessibilityAction
        ))
        .accessibilityIdentifier(CloudBackupUiElement.walletRow(recordId: item.recordId).identifier)
    }

    private var name: some View {
        Text(item.name)
            .fontWeight(.medium)
            .uiTestTruncationProbe()
    }
}

private struct WalletItemMetadata: View {
    let item: CloudBackupWalletItem

    var body: some View {
        WalletIdentityMetadata(item: item)
        WalletBackupMetadata(item: item)
    }
}

/// Metadata items side by side, or one under another at accessibility sizes, where side by side
/// each item wrapped into a column a word or two wide
private struct WalletMetadataLayout<Content: View>: View {
    @Environment(\.dynamicTypeSize) private var dynamicTypeSize
    @ViewBuilder let content: Content

    var body: some View {
        let layout = dynamicTypeSize.isAccessibilitySize
            ? AnyLayout(VStackLayout(alignment: .leading, spacing: 4))
            : AnyLayout(HStackLayout(spacing: 12))

        layout {
            content
        }
        .font(.caption)
        .foregroundStyle(.secondary)
    }
}

private struct WalletIdentityMetadata: View {
    let item: CloudBackupWalletItem

    var body: some View {
        WalletMetadataLayout {
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
    }
}

private struct WalletBackupMetadata: View {
    let item: CloudBackupWalletItem

    var body: some View {
        WalletMetadataLayout {
            if let labelCount = item.labelCount {
                IconLabel("tag", "\(labelCount) labels")
            }
            if let backupUpdatedAt = item.backupUpdatedAt {
                IconLabel("clock", Date.formattedTimestamp(backupUpdatedAt))
            }
        }
    }
}

func cloudBackupWalletAccessibilityLabel(
    item: CloudBackupWalletItem,
    action: String? = nil
) -> String {
    var components = [item.name]

    if let network = item.network {
        components.append(network.displayName())
    }

    components.append(item.syncStatus.accessibilityLabel)

    if let failure = item.restoreFailure {
        components.append("Restore failed: \(failure.message)")
    }

    if let action {
        components.append(action)
    }

    return components.joined(separator: ", ")
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

    private var color: Color {
        switch status {
        case .dirty: .statusWarning
        case .uploading, .uploadedPendingConfirmation: .statusInfo
        case .confirmed: .statusSuccess
        case .failed: .statusError
        case .deletedFromDevice, .unsupportedVersion: .statusWarning
        case .alreadyOnDevice: .statusInfo
        case .remoteStateUnknown: .secondary
        }
    }

    var body: some View {
        Text(status.accessibilityLabel)
            .font(.caption)
            .fontWeight(.medium)
            .uiTestTruncationProbe()
            .foregroundColor(color)
            .padding(.horizontal, 8)
            .padding(.vertical, 3)
            .background(color.opacity(0.15), in: Capsule())
    }
}

private extension CloudBackupWalletStatus {
    var accessibilityLabel: String {
        switch self {
        case .dirty: "Dirty"
        case .uploading: "Uploading"
        case .uploadedPendingConfirmation: "Uploaded, confirming"
        case .confirmed: "Confirmed"
        case .failed: "Failed"
        case .deletedFromDevice: "Not on device"
        case .alreadyOnDevice: "Already on this device"
        case .unsupportedVersion: "Unsupported, requires a newer version of Cove"
        case .remoteStateUnknown: "Unknown"
        }
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
