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
        OtherBackupsFinalDeleteAlert(
            manager: manager,
            isPresented: $showingFinalDeleteConfirmation,
            content: OtherBackupsDeleteConfirmationAlert(
                manager: manager,
                isPresented: $showingDeleteConfirmation,
                showFinalConfirmation: $showingFinalDeleteConfirmation,
                content: OtherBackupsRecoveryResultAlert(
                    manager: manager,
                    recoveryResult: $recoveryResult,
                    content: OtherBackupsRecoverConfirmationDialog(
                        manager: manager,
                        isPresented: $showingRecoverConfirmation,
                        content: OtherBackupsSectionContent(
                            summaryText: summaryText,
                            isRecovering: isRecovering,
                            isDeleting: isDeleting,
                            isOperating: isOperating,
                            isInventoryComplete: manager.isDetailInventoryComplete,
                            failure: failure,
                            onRequestRecovery: requestRecovery,
                            onRequestDeletion: requestDeletion
                        )
                    )
                )
            )
        )
        .onChange(of: manager.otherBackupsOperation) { _, operation in
            handleOperationChange(operation)
        }
    }

    private func requestRecovery() {
        guard manager.isDetailInventoryComplete else { return }

        showingRecoverConfirmation = true
    }

    private func requestDeletion() {
        guard manager.isDetailInventoryComplete else { return }

        showingDeleteConfirmation = true
    }

    private func handleOperationChange(_ operation: OtherBackupsOperation) {
        if case let .recovered(walletsRestored, walletsFailed, failedWalletErrors) = operation {
            recoveryResult = OtherBackupsRecoveryResult(
                walletsRestored: walletsRestored,
                walletsFailed: walletsFailed,
                failedWalletErrors: failedWalletErrors
            )
        }
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
        Section(header: Text("Other Cloud Backups")) {
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

private struct OtherBackupsRecoverConfirmationDialog<Content: View>: View {
    let manager: CloudBackupManager
    @Binding var isPresented: Bool
    let content: Content

    var body: some View {
        content.confirmationDialog(
            "Recover wallets from another passkey?",
            isPresented: $isPresented,
            titleVisibility: .visible
        ) {
            Button("Try Passkey", action: recoverOtherBackups)
                .disabled(!manager.isDetailInventoryComplete)

            Button("Cancel", role: .cancel) {}
        } message: {
            Text(
                "This will use the selected passkey once to decrypt these other backups. Your current Cloud Backup passkey will not change."
            )
        }
    }

    private func recoverOtherBackups() {
        guard manager.isDetailInventoryComplete else { return }

        manager.dispatch(action: .recoverOtherBackups)
    }
}

private struct OtherBackupsRecoveryResultAlert<Content: View>: View {
    let manager: CloudBackupManager
    @Binding var recoveryResult: OtherBackupsRecoveryResult?
    let content: Content

    private var isPresented: Binding<Bool> {
        Binding(
            get: { recoveryResult != nil },
            set: { if !$0 { recoveryResult = nil } }
        )
    }

    var body: some View {
        content.alert(
            "Wallets Recovered",
            isPresented: isPresented
        ) {
            Button("Verify Current Passkey", action: verifyCurrentPasskey)
            Button("Done", role: .cancel) {}
        } message: {
            Text(recoveryResult?.message ?? "")
        }
    }

    private func verifyCurrentPasskey() {
        manager.startVerification(source: .cloudBackupDetail)
    }
}

private struct OtherBackupsDeleteConfirmationAlert<Content: View>: View {
    let manager: CloudBackupManager
    @Binding var isPresented: Bool
    @Binding var showFinalConfirmation: Bool
    let content: Content

    var body: some View {
        content.alert("Delete Other Cloud Backups?", isPresented: $isPresented) {
            Button("Continue", role: .destructive, action: continueDeletion)
                .disabled(!manager.isDetailInventoryComplete)

            Button("Cancel", role: .cancel) {}
        } message: {
            Text("This will permanently remove these other backups from iCloud.")
        }
    }

    private func continueDeletion() {
        guard manager.isDetailInventoryComplete else { return }

        showFinalConfirmation = true
    }
}

private struct OtherBackupsFinalDeleteAlert<Content: View>: View {
    let manager: CloudBackupManager
    @Binding var isPresented: Bool
    let content: Content

    var body: some View {
        content.alert("This Cannot Be Undone", isPresented: $isPresented) {
            Button("Delete", role: .destructive, action: deleteOtherBackups)
                .disabled(!manager.isDetailInventoryComplete)

            Button("Cancel", role: .cancel) {}
        } message: {
            Text(
                "These backups cannot be recovered later, even if you find the passkey that currently protects them."
            )
        }
    }

    private func deleteOtherBackups() {
        guard manager.isDetailInventoryComplete else { return }

        manager.dispatch(action: .deleteOtherBackups)
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
