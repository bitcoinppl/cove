import SwiftUI

struct OtherBackupsSection: View {
    let summary: CloudBackupOtherBackupsSummary
    let manager: CloudBackupManager
    let presentationCoordinator: PresentationTransitionCoordinator<CloudBackupDetailPresentation>

    private var isRecovering: Bool {
        if case .recovering = manager.otherBackupsOperation { return true }
        return false
    }

    private var isDeleting: Bool {
        if case .deleting = manager.otherBackupsOperation { return true }
        return false
    }

    private var isOperating: Bool {
        isRecovering || isDeleting
    }

    private var failure: String? {
        if case let .failed(error) = manager.otherBackupsOperation { return error }
        return nil
    }

    private var summaryText: String {
        let namespaceLabel = pluralize(Int(summary.namespaceCount), singular: "backup set", plural: "backup sets")
        let walletLabel = pluralize(Int(summary.walletCount), singular: "wallet", plural: "wallets")
        let passkeyLabel = otherPasskeyLabel
        return "\(namespaceLabel) protected by \(passkeyLabel), containing \(walletLabel)"
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
        OtherBackupsSectionContent(
            summaryText: summaryText,
            isRecovering: isRecovering,
            isDeleting: isDeleting,
            isOperating: isOperating,
            isInventoryComplete: manager.isOtherBackupsInventoryReady,
            failure: failure,
            onRequestRecovery: requestRecovery,
            onRequestDeletion: requestDeletion
        )
    }

    private func requestRecovery() {
        guard manager.isOtherBackupsInventoryReady else { return }

        presentationCoordinator.present(.dialog(.recoverOtherBackups))
    }

    private func requestDeletion() {
        guard manager.isOtherBackupsInventoryReady else { return }

        presentationCoordinator.present(.alert(.otherBackupsDeleteConfirmation))
    }
}

private struct OtherBackupsSectionContent: View {
    let summaryText: String
    let isRecovering: Bool
    let isDeleting: Bool
    let isOperating: Bool
    let isInventoryComplete: Bool
    let failure: String?
    let onRequestRecovery: () -> Void
    let onRequestDeletion: () -> Void

    var body: some View {
        Section(header: Text("Backups with Another Key")) {
            Text(summaryText)
                .font(.caption)
                .foregroundStyle(.secondary)

            Button(action: onRequestRecovery) {
                OtherBackupsOperationLabel(
                    title: isRecovering ? "Trying Passkey..." : "Try Another Passkey",
                    systemImage: "person.badge.key",
                    isLoading: isRecovering
                )
            }
            .disabled(isOperating || !isInventoryComplete)

            Button(role: .destructive, action: onRequestDeletion) {
                OtherBackupsOperationLabel(
                    title: isDeleting ? "Deleting..." : "Delete These Backups",
                    systemImage: "trash",
                    isLoading: isDeleting
                )
            }
            .disabled(isOperating || !isInventoryComplete)

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
