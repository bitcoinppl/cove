import SwiftUI

/// Backups made with another passkey, and whether Cove could check for them, with the controls
/// Rust names for the section
struct OtherBackupsSection: View {
    let section: CloudBackupOtherBackupsSection
    let actions: CloudBackupActions
    let manager: CloudBackupManager
    let presentationCoordinator: PresentationTransitionCoordinator<CloudBackupDetailPresentation>

    var body: some View {
        switch section.state {
        case .notChecked:
            EmptyView()

        case .checking:
            OtherBackupsCheckingSection()

        case let .loaded(summary):
            // a check that found none has nothing to list
            if summary.namespaceCount > 0 {
                OtherBackupsSectionContent(summary: summary, operation: section.operation) {
                    controls
                }
            }

        case let .loadFailed(reason):
            OtherBackupsLoadFailedSection(reason: reason) {
                controls
            }
        }
    }

    private var controls: some View {
        OtherBackupsControls(
            section: section,
            actions: actions,
            manager: manager,
            presentationCoordinator: presentationCoordinator
        )
    }
}

/// The controls Rust names for the section, in its order
private struct OtherBackupsControls: View {
    let section: CloudBackupOtherBackupsSection
    let actions: CloudBackupActions
    let manager: CloudBackupManager
    let presentationCoordinator: PresentationTransitionCoordinator<CloudBackupDetailPresentation>

    var body: some View {
        ForEach(section.controls, id: \.self) { control in
            OtherBackupsControlButton(
                control: control,
                operation: section.operation,
                actions: actions,
                manager: manager,
                presentationCoordinator: presentationCoordinator
            )
        }
    }
}

/// One control of the section, drawn from its action's availability
private struct OtherBackupsControlButton: View {
    let control: CloudBackupOtherBackupsControl
    let operation: OtherBackupsOperation
    let actions: CloudBackupActions
    let manager: CloudBackupManager
    let presentationCoordinator: PresentationTransitionCoordinator<CloudBackupDetailPresentation>

    var body: some View {
        switch control {
        case .recover:
            CloudBackupActionControl(
                action: .recoverOtherBackups,
                actions: actions,
                perform: requestRecovery
            ) {
                OtherBackupsOperationLabel(
                    title: operation == .recovering ? "Trying Passkey..." : "Try Another Passkey",
                    systemImage: "person.badge.key",
                    isLoading: operation == .recovering
                )
            }

        case .delete:
            CloudBackupActionControl(
                action: .deleteOtherBackups,
                actions: actions,
                role: .destructive,
                perform: requestDeletion
            ) {
                OtherBackupsOperationLabel(
                    title: operation == .deleting ? "Deleting..." : "Delete These Backups",
                    systemImage: "trash",
                    isLoading: operation == .deleting
                )
            }

        case .checkAgain:
            CloudBackupActionControl(
                action: .refreshDetail,
                actions: actions,
                perform: checkAgain
            ) {
                Text("Check Again")
            }
        }
    }

    private func requestRecovery() {
        presentationCoordinator.present(.dialog(.recoverOtherBackups))
    }

    private func requestDeletion() {
        presentationCoordinator.present(.alert(.otherBackupsDeleteConfirmation))
    }

    private func checkAgain() {
        manager.dispatch(action: .refreshDetail)
    }
}

private struct OtherBackupsCheckingSection: View {
    var body: some View {
        Section(header: Text("Backups with Another Key")) {
            HStack {
                ProgressView()
                    .padding(.trailing, 8)
                Text("Checking for backups with another key...")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
        }
    }
}

private struct OtherBackupsLoadFailedSection<Controls: View>: View {
    let reason: CloudBackupInventoryIncompleteReason
    @ViewBuilder let controls: Controls

    var body: some View {
        Section(header: Text("Backups with Another Key")) {
            Text(reason.otherBackupsMessage)
                .font(.caption)
                .foregroundStyle(Color.statusError)

            controls
        }
    }
}

private struct OtherBackupsSectionContent<Controls: View>: View {
    let summary: CloudBackupOtherBackupsSummary
    let operation: OtherBackupsOperation
    @ViewBuilder let controls: Controls

    private var failure: String? {
        if case let .failed(error) = operation { return error }
        return nil
    }

    private var summaryText: String {
        let namespaceLabel = pluralize(Int(summary.namespaceCount), singular: "backup set", plural: "backup sets")
        let walletLabel = pluralize(Int(summary.walletCount), singular: "wallet", plural: "wallets")
        return "\(namespaceLabel) protected by \(otherPasskeyLabel), containing \(walletLabel)"
    }

    private var otherPasskeyLabel: String {
        let suffixes = summary.passkeyHints.map(\.nameSuffix)

        guard !suffixes.isEmpty else {
            return "a different passkey"
        }

        if suffixes.count == 1 {
            return "Cove Cloud Backup (\(suffixes[0]))"
        }

        return "passkeys \(suffixes.map { "(\($0))" }.joined(separator: ", "))"
    }

    var body: some View {
        Section(header: Text("Backups with Another Key")) {
            Text(summaryText)
                .font(.caption)
                .foregroundStyle(.secondary)

            controls

            if let failure {
                Text(failure)
                    .font(.caption)
                    .foregroundStyle(Color.statusError)
            }
        }
    }
}

private struct OtherBackupsOperationLabel: View {
    let title: String
    let systemImage: String
    let isLoading: Bool

    var body: some View {
        HStack {
            if isLoading {
                ProgressView()
                    .padding(.trailing, 4)
            } else {
                Image(systemName: systemImage)
            }
            Text(title)
        }
    }
}

struct OtherBackupsRecoveryResult {
    let walletsRestored: UInt32
    let walletsFailed: UInt32
    let failedWalletErrors: [String]

    var message: String {
        var parts = [
            "Recovered \(pluralize(Int(walletsRestored), singular: "wallet", plural: "wallets")).",
            "Your current Cloud Backup passkey is unchanged. Verify your current passkey to make sure it opens your active backup.",
        ]

        if walletsFailed > 0 {
            parts.append(
                "\(pluralize(Int(walletsFailed), singular: "wallet", plural: "wallets")) could not be recovered."
            )
        }

        if let firstError = failedWalletErrors.first {
            parts.append(firstError)
        }

        return parts.joined(separator: " ")
    }
}

private func pluralize(_ count: Int, singular: String, plural: String) -> String {
    "\(count) \(count == 1 ? singular : plural)"
}
