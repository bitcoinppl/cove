import SwiftUI

@_exported import CoveCore

private extension CloudBackupVerificationState? {
    var isVerifying: Bool {
        if case .running = self { return true }
        return false
    }

    var hasResult: Bool {
        switch self {
        case .verified, .needsAttention, .awaitingUploadConfirmation, .cancelled, .failed: true
        default: false
        }
    }
}

private extension CloudBackupPasskeyRepairState? {
    var isRecovering: Bool {
        if case .running = self { return true }
        return false
    }
}

struct VerificationSection: View {
    let manager: CloudBackupManager
    let presentationCoordinator: PresentationTransitionCoordinator<CloudBackupDetailPresentation>
    let recreateConfirmationIsPresented: Binding<Bool>
    let reinitializeConfirmationIsPresented: Binding<Bool>

    private var undecryptableWalletCount: UInt32 {
        guard case let .needsAttention(report: report, checkedAt: _) = manager.verificationState else {
            return 0
        }

        return report.walletIssues.decryptionFailed
    }

    private var isBusy: Bool {
        manager.verificationState.isVerifying ||
            manager.passkeyRepairState.isRecovering ||
            manager.isPerformingDestructiveAction
    }

    var body: some View {
        content
    }

    @ViewBuilder
    private var content: some View {
        switch manager.verificationState {
        case nil, .notVerified, .required:
            CloudBackupVerificationStartSection(
                isBusy: isBusy,
                onVerify: startVerification
            )
        case .running:
            EmptyView()
        case let .verified(report: report, lastVerifiedAt: _):
            if let report {
                CloudBackupVerifiedSection(
                    report: report,
                    isBusy: isBusy,
                    needsSync: manager.detail?.needsSync.isEmpty == false,
                    syncState: manager.syncState,
                    onSync: syncUnsynced,
                    onVerify: startVerification
                )
            } else {
                CloudBackupPasskeyConfirmedSection(
                    isBusy: isBusy,
                    onVerify: startVerification
                )
            }
        case let .needsAttention(report: report, checkedAt: _):
            CloudBackupNeedsAttentionSection(
                report: report,
                isBusy: isBusy,
                needsSync: manager.detail?.needsSync.isEmpty == false,
                syncState: manager.syncState,
                onSync: syncUnsynced,
                onVerify: startVerification,
                deletionState: manager.undecryptableWalletDeletionState,
                onDeleteUndecryptable: requestUndecryptableWalletDeletion
            )
        case .awaitingUploadConfirmation:
            CloudBackupUploadConfirmationPendingSection()
        case .cancelled:
            CloudBackupVerificationCancelledSection(
                isBusy: isBusy,
                isRecoveringPasskey: manager.passkeyRepairState.isRecovering,
                onVerify: startVerification,
                onRepairPasskey: repairPasskey
            )
        case let .failed(failure):
            CloudBackupVerificationFailureSection(
                failure: failure,
                passkeyRepairState: manager.passkeyRepairState,
                isBusy: isBusy,
                isDetailInventoryComplete: manager.isDetailInventoryComplete,
                destructiveOperationState: manager.destructiveOperationState,
                onRetry: retry,
                onRepairPasskey: repairPasskey,
                recreateConfirmationIsPresented: recreateConfirmationIsPresented,
                reinitializeConfirmationIsPresented: reinitializeConfirmationIsPresented
            )
        }
    }

    private func startVerification() {
        manager.startVerification()
    }

    private func retry(_ retryAction: CloudBackupRetryAction?) {
        if retryAction == .verifyDiscoverable {
            manager.dispatch(action: .startVerificationDiscoverable(.cloudBackupDetail))
        } else {
            manager.startVerification(source: .cloudBackupDetail)
        }
    }

    private func repairPasskey() {
        manager.dispatch(action: .repairPasskeyNoDiscovery)
    }

    private func syncUnsynced() {
        manager.dispatch(action: .syncUnsynced)
    }

    private func requestUndecryptableWalletDeletion() {
        guard undecryptableWalletCount > 0 else { return }

        presentationCoordinator.present(
            .alert(.undecryptableWalletDeletion(undecryptableWalletCount))
        )
    }
}

private struct CloudBackupVerificationStartSection: View {
    let isBusy: Bool
    let onVerify: () -> Void

    var body: some View {
        Section {
            Text("Run verification to confirm your cloud backup can be decrypted and restored")
                .font(.caption)
                .foregroundStyle(.secondary)

            Button(action: onVerify) {
                Label("Verify Now", systemImage: "checkmark.shield")
            }
            .disabled(isBusy)
        }
    }
}

private struct CloudBackupVerificationCancelledSection: View {
    let isBusy: Bool
    let isRecoveringPasskey: Bool
    let onVerify: () -> Void
    let onRepairPasskey: () -> Void

    var body: some View {
        Section {
            Label(
                "Cloud Backup Not Verified",
                systemImage: "exclamationmark.shield.fill"
            )
            .foregroundStyle(Color.statusWarning)

            Text(
                "If your passkey was deleted, add a new one. Otherwise, verify again with your current passkey."
            )
            .font(.caption)
            .foregroundStyle(.secondary)

            Button(action: onVerify) {
                Label("Verify Now", systemImage: "checkmark.shield")
            }
            .disabled(isBusy)

            CloudBackupRepairPasskeyButton(
                isBusy: isBusy,
                isRecovering: isRecoveringPasskey,
                onRepair: onRepairPasskey
            )
        }
    }
}

private struct CloudBackupVerifiedSection: View {
    let report: DeepVerificationReport
    let isBusy: Bool
    let needsSync: Bool
    let syncState: CloudBackupSyncState?
    let onSync: () -> Void
    let onVerify: () -> Void

    private var summary: String? {
        var parts: [String] = []

        if report.credentialRecovered {
            parts.append("Passkey recovered")
        }

        if report.masterKeyWrapperRepaired {
            parts.append("Cloud master key protection repaired")
        }

        if report.localMasterKeyRepaired {
            parts.append("Local backup credentials repaired")
        }

        if report.walletsVerified > 0 {
            parts.append("\(report.walletsVerified) wallet(s) verified")
        }

        return parts.isEmpty ? nil : parts.joined(separator: ", ")
    }

    var body: some View {
        Section {
            Label("Backup verified", systemImage: "checkmark.shield.fill")
                .foregroundStyle(Color.statusSuccess)
                .alignmentGuide(.listRowSeparatorLeading) { _ in 0 }

            if let summary {
                Text(summary)
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
        }

        CloudBackupVerificationActionButtons(
            isBusy: isBusy,
            needsSync: needsSync,
            syncState: syncState,
            onSync: onSync,
            onVerify: onVerify
        )
    }
}

private struct CloudBackupNeedsAttentionSection: View {
    let report: DeepVerificationReport
    let isBusy: Bool
    let needsSync: Bool
    let syncState: CloudBackupSyncState?
    let onSync: () -> Void
    let onVerify: () -> Void
    let deletionState: CloudBackupUndecryptableWalletDeletionState
    let onDeleteUndecryptable: () -> Void

    var body: some View {
        Section {
            Label("Backup needs attention", systemImage: "exclamationmark.shield.fill")
                .foregroundStyle(Color.statusWarning)
                .alignmentGuide(.listRowSeparatorLeading) { _ in 0 }

            Text("The backup key is valid. \(report.walletsVerified) wallet backup(s) passed verification.")
                .font(.caption)
                .foregroundStyle(.secondary)

            CloudBackupWalletIssueRows(
                issues: report.walletIssues,
                deletionState: deletionState,
                onDeleteUndecryptable: onDeleteUndecryptable
            )
        }

        CloudBackupVerificationActionButtons(
            isBusy: isBusy,
            needsSync: needsSync,
            syncState: syncState,
            onSync: onSync,
            onVerify: onVerify
        )
    }
}

private struct CloudBackupWalletIssueRows: View {
    let issues: CloudBackupWalletVerificationIssues
    let deletionState: CloudBackupUndecryptableWalletDeletionState
    let onDeleteUndecryptable: () -> Void

    var body: some View {
        issue(issues.missing, "wallet backup file(s) are missing from cloud storage")
        issue(issues.downloadFailed, "wallet backup(s) could not be downloaded")
        issue(issues.invalid, "wallet backup file(s) contain invalid data")
        undecryptableIssue()
        issue(issues.unsupported, "wallet backup(s) use a newer backup format")
        issue(issues.unreadable, "wallet backup(s) could not be read")

        if case let .failed(error) = deletionState {
            Label(error, systemImage: "xmark.circle.fill")
                .foregroundStyle(Color.statusError)
                .font(.caption)
        }
    }

    @ViewBuilder
    private func undecryptableIssue() -> some View {
        if issues.decryptionFailed > 0 {
            Button(role: .destructive, action: onDeleteUndecryptable) {
                HStack {
                    if case .deleting = deletionState {
                        ProgressView()
                    } else {
                        Image(systemName: "exclamationmark.triangle.fill")
                    }

                    Text(
                        "\(issues.decryptionFailed) wallet backup(s) could not be decrypted with this backup key"
                    )
                    Spacer()
                    Image(systemName: "chevron.right")
                        .font(.caption)
                }
            }
            .font(.caption)
            .disabled(deletionState == .deleting)
            .accessibilityHint("Opens a confirmation to delete these inaccessible backups")
        }
    }

    @ViewBuilder
    private func issue(_ count: UInt32, _ message: String) -> some View {
        if count > 0 {
            Label("\(count) \(message)", systemImage: "exclamationmark.triangle.fill")
                .foregroundStyle(Color.statusError)
                .font(.caption)
        }
    }
}

private struct CloudBackupVerificationFailureSection: View {
    let failure: DeepVerificationFailure
    let passkeyRepairState: CloudBackupPasskeyRepairState?
    let isBusy: Bool
    let isDetailInventoryComplete: Bool
    let destructiveOperationState: CloudBackupDestructiveOperationState
    let onRetry: (CloudBackupRetryAction?) -> Void
    let onRepairPasskey: () -> Void
    let recreateConfirmationIsPresented: Binding<Bool>
    let reinitializeConfirmationIsPresented: Binding<Bool>

    private var passkeyRepairError: String? {
        guard case let .failed(error) = passkeyRepairState else { return nil }

        return error
    }

    var body: some View {
        Section {
            switch failure {
            case let .retry(message, _, retryAction):
                CloudBackupRetryFailureContent(
                    message: message,
                    retryAction: retryAction,
                    isBusy: isBusy,
                    isRecoveringPasskey: passkeyRepairState.isRecovering,
                    onRetry: onRetry,
                    onRepairPasskey: onRepairPasskey
                )
            case let .recreateManifest(message, warning, _):
                CloudBackupRecreateManifestFailureContent(
                    message: message,
                    warning: warning,
                    isBusy: isBusy,
                    isDetailInventoryComplete: isDetailInventoryComplete,
                    destructiveOperationState: destructiveOperationState,
                    confirmationIsPresented: recreateConfirmationIsPresented
                )
            case let .reinitializeBackup(message, warning, _):
                CloudBackupReinitializeFailureContent(
                    message: message,
                    warning: warning,
                    isBusy: isBusy,
                    isDetailInventoryComplete: isDetailInventoryComplete,
                    destructiveOperationState: destructiveOperationState,
                    confirmationIsPresented: reinitializeConfirmationIsPresented
                )
            case let .unsupportedVersion(message, _):
                CloudBackupUnsupportedVersionFailureContent(message: message)
            }
        }

        CloudBackupPasskeyRepairErrorSection(error: passkeyRepairError)
    }
}

private struct CloudBackupRetryFailureContent: View {
    let message: String
    let retryAction: CloudBackupRetryAction?
    let isBusy: Bool
    let isRecoveringPasskey: Bool
    let onRetry: (CloudBackupRetryAction?) -> Void
    let onRepairPasskey: () -> Void

    var body: some View {
        Label(message, systemImage: "exclamationmark.triangle.fill")
            .foregroundStyle(Color.statusWarning)

        Button {
            onRetry(retryAction)
        } label: {
            Label("Try Again", systemImage: "arrow.clockwise")
        }
        .disabled(isBusy)

        CloudBackupRepairPasskeyButton(
            isBusy: isBusy,
            isRecovering: isRecoveringPasskey,
            onRepair: onRepairPasskey
        )
    }
}

private struct CloudBackupRecreateManifestFailureContent: View {
    let message: String
    let warning: String
    let isBusy: Bool
    let isDetailInventoryComplete: Bool
    let destructiveOperationState: CloudBackupDestructiveOperationState
    let confirmationIsPresented: Binding<Bool>

    var body: some View {
        Label(message, systemImage: "exclamationmark.triangle.fill")
            .foregroundStyle(Color.statusError)

        Text(warning)
            .font(.caption)
            .foregroundStyle(.secondary)

        CloudBackupDestructiveActionButton(
            title: "Recreate Backup Index",
            progressTitle: "Recreating...",
            systemImage: "arrow.clockwise",
            operation: .recreatingManifest,
            currentOperation: destructiveOperationState,
            isBusy: isBusy,
            isDetailInventoryComplete: isDetailInventoryComplete,
            action: { confirmationIsPresented.wrappedValue = true }
        )
    }
}

private struct CloudBackupReinitializeFailureContent: View {
    let message: String
    let warning: String
    let isBusy: Bool
    let isDetailInventoryComplete: Bool
    let destructiveOperationState: CloudBackupDestructiveOperationState
    let confirmationIsPresented: Binding<Bool>

    var body: some View {
        Label(message, systemImage: "exclamationmark.triangle.fill")
            .foregroundStyle(Color.statusError)

        Text(warning)
            .font(.caption)
            .foregroundStyle(.secondary)

        CloudBackupDestructiveActionButton(
            title: "Reinitialize Cloud Backup",
            progressTitle: "Reinitializing...",
            systemImage: "arrow.counterclockwise",
            operation: .reinitializingBackup,
            currentOperation: destructiveOperationState,
            isBusy: isBusy,
            isDetailInventoryComplete: isDetailInventoryComplete,
            action: { confirmationIsPresented.wrappedValue = true }
        )
    }
}

private struct CloudBackupUnsupportedVersionFailureContent: View {
    let message: String

    var body: some View {
        Label(message, systemImage: "exclamationmark.triangle.fill")
            .foregroundStyle(Color.statusWarning)

        Text("Please update the app to the latest version")
            .font(.caption)
            .foregroundStyle(.secondary)
    }
}

private struct CloudBackupDestructiveActionButton: View {
    let title: String
    let progressTitle: String
    let systemImage: String
    let operation: CloudBackupDestructiveOperationState
    let currentOperation: CloudBackupDestructiveOperationState
    let isBusy: Bool
    let isDetailInventoryComplete: Bool
    let action: () -> Void

    var body: some View {
        Button(role: .destructive) {
            guard isDetailInventoryComplete else { return }

            action()
        } label: {
            if currentOperation == operation {
                HStack {
                    ProgressView()
                        .padding(.trailing, 4)
                    Text(progressTitle)
                }
            } else {
                Label(title, systemImage: systemImage)
            }
        }
        .disabled(isBusy || !isDetailInventoryComplete)
    }
}

private struct CloudBackupPasskeyRepairErrorSection: View {
    let error: String?

    var body: some View {
        if let error {
            Section {
                Label(error, systemImage: "xmark.circle.fill")
                    .foregroundStyle(Color.statusError)
                    .font(.caption)
            }
        }
    }
}

private struct CloudBackupUploadConfirmationPendingSection: View {
    var body: some View {
        Section {
            Label("Cloud Backup enabled", systemImage: "icloud.and.arrow.up.fill")
                .foregroundStyle(Color.statusSuccess)

            Text("Cove is still confirming that your encrypted backup is visible in iCloud. You can leave this screen while confirmation continues in the background.")
                .font(.caption)
                .foregroundStyle(.secondary)
        }
    }
}

private struct CloudBackupPasskeyConfirmedSection: View {
    let isBusy: Bool
    let onVerify: () -> Void

    var body: some View {
        Section {
            Label("Passkey verified", systemImage: "checkmark.shield.fill")
                .foregroundStyle(Color.statusSuccess)

            Text("Your stored passkey is valid. Run a full verification to confirm wallet backups can be decrypted.")
                .font(.caption)
                .foregroundStyle(.secondary)

            Button(action: onVerify) {
                Label("Run Full Verification", systemImage: "checkmark.shield")
            }
            .disabled(isBusy)
        }
    }
}

private struct CloudBackupVerificationActionButtons: View {
    let isBusy: Bool
    let needsSync: Bool
    let syncState: CloudBackupSyncState?
    let onSync: () -> Void
    let onVerify: () -> Void

    var body: some View {
        Section {
            if needsSync {
                CloudBackupVerificationSyncButton(
                    syncState: syncState,
                    onSync: onSync
                )
            }

            Button(action: onVerify) {
                Label("Verify Again", systemImage: "checkmark.shield")
            }
            .disabled(isBusy)
        }
    }
}

private struct CloudBackupVerificationSyncButton: View {
    let syncState: CloudBackupSyncState?
    let onSync: () -> Void

    var body: some View {
        Button(action: onSync) {
            HStack {
                if case .syncing = syncState {
                    ProgressView()
                        .padding(.trailing, 8)
                    Text("Syncing...")
                } else {
                    Image(systemName: "arrow.triangle.2.circlepath")
                    Text("Sync Now")
                }
            }
        }
        .disabled(syncState == .syncing)

        if case let .failed(error) = syncState {
            Text(error)
                .font(.caption)
                .foregroundStyle(Color.statusError)
        }
    }
}

private struct CloudBackupRepairPasskeyButton: View {
    let isBusy: Bool
    let isRecovering: Bool
    let onRepair: () -> Void

    var body: some View {
        Button(action: onRepair) {
            if isRecovering {
                HStack {
                    ProgressView()
                        .padding(.trailing, 4)
                    Text("Creating Passkey...")
                }
            } else {
                Label("Add New Passkey", systemImage: "person.badge.key")
            }
        }
        .disabled(isBusy)
    }
}
