import SwiftUI

struct OtherBackupsSection: View {
    let summary: CloudBackupOtherBackupsSummary
    let manager: CloudBackupManager
    @State private var showingRecoverConfirmation = false
    @State private var showingDeleteConfirmation = false
    @State private var showingFinalDeleteConfirmation = false
    @State private var recoveryResult: OtherBackupsRecoveryResult?

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

    var body: some View {
        Section(header: Text("Other Cloud Backups")) {
            Text(summaryText)
                .font(.caption)
                .foregroundStyle(.secondary)

            Button {
                guard manager.isDetailInventoryComplete else { return }

                showingRecoverConfirmation = true
            } label: {
                operationLabel(
                    title: isRecovering ? "Trying Passkey..." : "Try Another Passkey",
                    systemImage: "person.badge.key",
                    isLoading: isRecovering
                )
            }
            .disabled(isOperating || !manager.isDetailInventoryComplete)

            Button(role: .destructive) {
                guard manager.isDetailInventoryComplete else { return }

                showingDeleteConfirmation = true
            } label: {
                operationLabel(
                    title: isDeleting ? "Deleting..." : "Delete These Backups",
                    systemImage: "trash",
                    isLoading: isDeleting
                )
            }
            .disabled(isOperating || !manager.isDetailInventoryComplete)

            if let failure {
                Text(failure)
                    .font(.caption)
                    .foregroundStyle(Color.statusError)
            }
        }
        .confirmationDialog(
            "Recover wallets from another passkey?",
            isPresented: $showingRecoverConfirmation,
            titleVisibility: .visible
        ) {
            Button("Try Passkey") {
                guard manager.isDetailInventoryComplete else { return }

                manager.dispatch(action: .recoverOtherBackups)
            }
            .disabled(!manager.isDetailInventoryComplete)

            Button("Cancel", role: .cancel) {}
        } message: {
            Text(
                "This will use the selected passkey once to decrypt these other backups. Your current Cloud Backup passkey will not change."
            )
        }
        .alert(
            "Wallets Recovered",
            isPresented: Binding(
                get: { recoveryResult != nil },
                set: { if !$0 { recoveryResult = nil } }
            )
        ) {
            Button("Verify Current Passkey") {
                manager.startVerification(source: .cloudBackupDetail)
            }
            Button("Done", role: .cancel) {}
        } message: {
            Text(recoveryResult?.message ?? "")
        }
        .alert("Delete Other Cloud Backups?", isPresented: $showingDeleteConfirmation) {
            Button("Continue", role: .destructive) {
                guard manager.isDetailInventoryComplete else { return }

                showingFinalDeleteConfirmation = true
            }
            .disabled(!manager.isDetailInventoryComplete)

            Button("Cancel", role: .cancel) {}
        } message: {
            Text("This will permanently remove these other backups from iCloud.")
        }
        .alert("This Cannot Be Undone", isPresented: $showingFinalDeleteConfirmation) {
            Button("Delete", role: .destructive) {
                guard manager.isDetailInventoryComplete else { return }

                manager.dispatch(action: .deleteOtherBackups)
            }
            .disabled(!manager.isDetailInventoryComplete)

            Button("Cancel", role: .cancel) {}
        } message: {
            Text(
                "These backups cannot be recovered later, even if you find the passkey that currently protects them."
            )
        }
        .onChange(of: manager.otherBackupsOperation) { _, operation in
            if case let .recovered(walletsRestored, walletsFailed, failedWalletErrors) = operation {
                recoveryResult = OtherBackupsRecoveryResult(
                    walletsRestored: walletsRestored,
                    walletsFailed: walletsFailed,
                    failedWalletErrors: failedWalletErrors
                )
            }
        }
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

    private func operationLabel(title: String, systemImage: String, isLoading: Bool) -> some View {
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

struct OtherBackupsLoadFailedSection: View {
    let error: String

    var body: some View {
        Section(header: Text("Other Cloud Backups")) {
            Text("Could not load other cloud backups.")
                .font(.caption)
                .foregroundStyle(.secondary)

            Text(error)
                .font(.caption)
                .foregroundStyle(Color.statusError)
        }
    }
}

private struct OtherBackupsRecoveryResult: Identifiable {
    let id = UUID()
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
