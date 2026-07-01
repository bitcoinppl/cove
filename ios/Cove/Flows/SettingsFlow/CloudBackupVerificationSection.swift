import SwiftUI

@_exported import CoveCore

private extension CloudBackupVerificationState? {
    var isVerifying: Bool {
        if case .running = self { return true }
        return false
    }

    var hasResult: Bool {
        switch self {
        case .verified, .awaitingUploadConfirmation, .failed: true
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
    let onRecreate: () -> Void
    let onReinitialize: () -> Void

    private var isBusy: Bool {
        manager.verificationState.isVerifying ||
            manager.passkeyRepairState.isRecovering ||
            manager.isPerformingDestructiveAction
    }

    var body: some View {
        switch manager.verificationState {
        case nil, .notVerified, .required:
            Section {
                Text("Run verification to confirm your cloud backup can be decrypted and restored")
                    .font(.caption)
                    .foregroundStyle(.secondary)

                Button {
                    manager.startVerification()
                } label: {
                    Label("Verify Now", systemImage: "checkmark.shield")
                }
                .disabled(isBusy)
            }
        case .running:
            Section {
                HStack {
                    ProgressView()
                        .padding(.trailing, 8)
                    Text("Verifying backup integrity...")
                }
            }
        case let .verified(report: report, lastVerifiedAt: _):
            if let report {
                verifiedSection(report)
            } else {
                CloudBackupPasskeyConfirmedSection(manager: manager, isBusy: isBusy)
            }
        case .awaitingUploadConfirmation:
            CloudBackupPasskeyConfirmedSection(manager: manager, isBusy: isBusy)
        case let .failed(failure):
            failureSection(failure)
        }
    }

    @ViewBuilder
    private func verifiedSection(_ report: DeepVerificationReport) -> some View {
        Section {
            Label("Backup verified", systemImage: "checkmark.shield.fill")
                .foregroundStyle(Color.statusSuccess)
                .alignmentGuide(.listRowSeparatorLeading) { _ in 0 }

            if let summary = verifiedSummary(report) {
                Text(summary)
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }

            if report.walletsFailed > 0 {
                Label(
                    failedWalletsMessage(report.walletsFailed),
                    systemImage: "exclamationmark.triangle.fill"
                )
                .foregroundStyle(Color.statusError)
                .font(.caption)
            }

            if report.walletsUnsupported > 0 {
                Label(
                    unsupportedWalletsMessage(report.walletsUnsupported),
                    systemImage: "info.circle.fill"
                )
                .foregroundStyle(Color.statusWarning)
                .font(.caption)
            }
        }

        CloudBackupVerificationActionButtons(manager: manager, isBusy: isBusy)
    }

    private func verifiedSummary(_ report: DeepVerificationReport) -> String? {
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
            parts.append(verifiedWalletsMessage(report.walletsVerified))
        }

        return parts.isEmpty ? nil : parts.joined(separator: ", ")
    }

    @ViewBuilder
    private func failureSection(_ failure: DeepVerificationFailure) -> some View {
        Section {
            switch failure {
            case let .retry(_, retryContext):
                retryFailureContent(failure.localizedMessage, retryContext: retryContext)
            case .recreateManifest:
                recreateManifestContent(
                    message: failure.localizedMessage,
                    warning: failure.localizedWarning ?? ""
                )
            case .reinitializeBackup:
                reinitializeBackupContent(
                    message: failure.localizedMessage,
                    warning: failure.localizedWarning ?? ""
                )
            case .unsupportedVersion:
                unsupportedVersionContent(failure.localizedMessage)
            }
        }

        if case .failed = manager.passkeyRepairState {
            Section {
                Label(String(localized: "Unable to repair passkey. Please try again."), systemImage: "xmark.circle.fill")
                    .foregroundStyle(Color.statusError)
                    .font(.caption)
            }
        }
    }

    @ViewBuilder
    private func retryFailureContent(_ message: String, retryContext: CloudBackupRetryContext?) -> some View {
        Label(message, systemImage: "exclamationmark.triangle.fill")
            .foregroundStyle(Color.statusWarning)

        retryButton(retryContext: retryContext)
        CloudBackupRepairPasskeyButton(manager: manager, isBusy: isBusy)
    }

    @ViewBuilder
    private func recreateManifestContent(message: String, warning: String) -> some View {
        Label(message, systemImage: "exclamationmark.triangle.fill")
            .foregroundStyle(Color.statusError)

        Text(warning)
            .font(.caption)
            .foregroundStyle(.secondary)

        destructiveActionButton(
            title: "Recreate Backup Index",
            progressTitle: "Recreating...",
            systemImage: "arrow.clockwise",
            operation: .recreatingManifest,
            action: onRecreate
        )
    }

    @ViewBuilder
    private func reinitializeBackupContent(message: String, warning: String) -> some View {
        Label(message, systemImage: "exclamationmark.triangle.fill")
            .foregroundStyle(Color.statusError)

        Text(warning)
            .font(.caption)
            .foregroundStyle(.secondary)

        destructiveActionButton(
            title: "Reinitialize Cloud Backup",
            progressTitle: "Reinitializing...",
            systemImage: "arrow.counterclockwise",
            operation: .reinitializingBackup,
            action: onReinitialize
        )
    }

    @ViewBuilder
    private func unsupportedVersionContent(_ message: String) -> some View {
        Label(message, systemImage: "exclamationmark.triangle.fill")
            .foregroundStyle(Color.statusWarning)

        Text("Please update the app to the latest version")
            .font(.caption)
            .foregroundStyle(.secondary)
    }

    private func destructiveActionButton(
        title: LocalizedStringKey,
        progressTitle: LocalizedStringKey,
        systemImage: String,
        operation: CloudBackupDestructiveOperationState,
        action: @escaping () -> Void
    ) -> some View {
        Button(role: .destructive) {
            action()
        } label: {
            if manager.destructiveOperationState == operation {
                HStack {
                    ProgressView()
                        .padding(.trailing, 4)
                    Text(progressTitle)
                }
            } else {
                Label(title, systemImage: systemImage)
            }
        }
        .disabled(isBusy)
    }

    private func retryButton(retryContext: CloudBackupRetryContext?) -> some View {
        Button {
            if retryContext?.action == .verifyDiscoverable {
                manager.dispatch(action: .startVerificationDiscoverable(.cloudBackupDetail))
            } else {
                manager.startVerification(source: .cloudBackupDetail)
            }
        } label: {
            Label("Try Again", systemImage: "arrow.clockwise")
        }
        .disabled(isBusy)
    }
}

private struct CloudBackupPasskeyConfirmedSection: View {
    let manager: CloudBackupManager
    let isBusy: Bool

    var body: some View {
        Section {
            Label("Passkey verified", systemImage: "checkmark.shield.fill")
                .foregroundStyle(Color.statusSuccess)

            Text("Your stored passkey is valid. Run a full verification to confirm wallet backups can be decrypted.")
                .font(.caption)
                .foregroundStyle(.secondary)

            Button {
                manager.startVerification()
            } label: {
                Label("Run Full Verification", systemImage: "checkmark.shield")
            }
            .disabled(isBusy)
        }
    }
}

private struct CloudBackupVerificationActionButtons: View {
    let manager: CloudBackupManager
    let isBusy: Bool

    var body: some View {
        Section {
            if manager.detail?.needsSync.isEmpty == false {
                CloudBackupVerificationSyncButton(manager: manager)
            }

            Button {
                manager.startVerification()
            } label: {
                Label("Verify Again", systemImage: "checkmark.shield")
            }
            .disabled(isBusy)
        }
    }
}

private struct CloudBackupVerificationSyncButton: View {
    let manager: CloudBackupManager

    var body: some View {
        Button {
            manager.dispatch(action: .syncUnsynced)
        } label: {
            HStack {
                if case .syncing = manager.syncState {
                    ProgressView()
                        .padding(.trailing, 8)
                    Text("Syncing...")
                } else {
                    Image(systemName: "arrow.triangle.2.circlepath")
                    Text("Sync Now")
                }
            }
        }
        .disabled(manager.syncState == .syncing)

        if case .failed = manager.syncState {
            Text("Unable to sync Cloud Backup. Please try again.")
                .font(.caption)
                .foregroundStyle(Color.statusError)
        }
    }
}

private func failedWalletsMessage(_ count: UInt32) -> String {
    if count == 1 {
        return String(localized: "1 wallet backup could not be decrypted")
    }

    return String(localized: "\(count) wallet backups could not be decrypted")
}

private func unsupportedWalletsMessage(_ count: UInt32) -> String {
    if count == 1 {
        return String(localized: "1 wallet uses a newer backup format")
    }

    return String(localized: "\(count) wallets use a newer backup format")
}

private func verifiedWalletsMessage(_ count: UInt32) -> String {
    if count == 1 {
        return String(localized: "1 wallet verified")
    }

    return String(localized: "\(count) wallets verified")
}

private struct CloudBackupRepairPasskeyButton: View {
    let manager: CloudBackupManager
    let isBusy: Bool

    var body: some View {
        Button {
            manager.dispatch(action: .repairPasskeyNoDiscovery)
        } label: {
            if manager.passkeyRepairState.isRecovering {
                HStack {
                    ProgressView()
                        .padding(.trailing, 4)
                    Text("Creating Passkey...")
                }
            } else {
                Label("Create New Passkey", systemImage: "person.badge.key")
            }
        }
        .disabled(isBusy)
    }
}
