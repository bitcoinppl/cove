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
                passkeyConfirmedSection
            }
        case .awaitingUploadConfirmation:
            passkeyConfirmedSection
        case let .failed(failure):
            failureSection(failure)
        }
    }

    private var passkeyConfirmedSection: some View {
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

    @ViewBuilder
    private func verifiedSection(_ report: DeepVerificationReport) -> some View {
        Section {
            Label("Backup verified", systemImage: "checkmark.shield.fill")
                .foregroundStyle(Color.statusSuccess)
                .alignmentGuide(.listRowSeparatorLeading) { _ in 0 }

            if report.masterKeyWrapperRepaired {
                Label(
                    "Cloud master key protection was repaired",
                    systemImage: "wrench.and.screwdriver.fill"
                )
                .foregroundStyle(Color.statusInfo)
                .font(.caption)
            }

            if report.localMasterKeyRepaired {
                Label(
                    "Local backup credentials were repaired from cloud",
                    systemImage: "wrench.and.screwdriver.fill"
                )
                .foregroundStyle(Color.statusInfo)
                .font(.caption)
            }

            if report.walletsFailed > 0 {
                Label(
                    "\(report.walletsFailed) wallet backup(s) could not be decrypted",
                    systemImage: "exclamationmark.triangle.fill"
                )
                .foregroundStyle(Color.statusError)
                .font(.caption)
            }

            if report.walletsUnsupported > 0 {
                Label(
                    "\(report.walletsUnsupported) wallet(s) use a newer backup format",
                    systemImage: "info.circle.fill"
                )
                .foregroundStyle(Color.statusWarning)
                .font(.caption)
            }

            if report.walletsVerified > 0 {
                Text("\(report.walletsVerified) wallet(s) verified")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
        }

        actionButtons
    }

    @ViewBuilder
    private func failureSection(_ failure: DeepVerificationFailure) -> some View {
        Section {
            switch failure {
            case let .retry(message, _, _):
                retryFailureContent(message)
            case let .recreateManifest(message, warning, _):
                recreateManifestContent(message: message, warning: warning)
            case let .reinitializeBackup(message, warning, _):
                reinitializeBackupContent(message: message, warning: warning)
            case let .unsupportedVersion(message, _):
                unsupportedVersionContent(message)
            }
        }

        if case let .failed(error) = manager.passkeyRepairState {
            Section {
                Label(error, systemImage: "xmark.circle.fill")
                    .foregroundStyle(Color.statusError)
                    .font(.caption)
            }
        }
    }

    @ViewBuilder
    private func retryFailureContent(_ message: String) -> some View {
        Label(message, systemImage: "exclamationmark.triangle.fill")
            .foregroundStyle(Color.statusWarning)

        retryButton
        repairPasskeyButton
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
        title: String,
        progressTitle: String,
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

    private var actionButtons: some View {
        Section {
            if manager.detail?.needsSync.isEmpty == false {
                syncButton
            }

            Button {
                manager.startVerification()
            } label: {
                Label("Verify Again", systemImage: "checkmark.shield")
            }
            .disabled(isBusy)
        }
    }

    private var syncButton: some View {
        Group {
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

            if case let .failed(error) = manager.syncState {
                Text(error)
                    .font(.caption)
                    .foregroundStyle(Color.statusError)
            }
        }
    }

    private var retryButton: some View {
        Button {
            manager.startVerification()
        } label: {
            Label("Try Again", systemImage: "arrow.clockwise")
        }
        .disabled(isBusy)
    }

    private var repairPasskeyButton: some View {
        Button {
            manager.dispatch(action: .repairPasskey)
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
