@_exported import CoveCore
import SwiftUI

/// The verification Rust reports, with the controls its section names
///
/// Each state draws its own content, and draws its controls from `section.controls` where that
/// state puts them; it never decides from the content which controls exist
struct VerificationSection: View {
    let section: CloudBackupVerificationSection
    let actions: CloudBackupActions
    let wallets: CloudBackupWalletsSection?
    let manager: CloudBackupManager
    let presentationCoordinator: PresentationTransitionCoordinator<CloudBackupDetailPresentation>

    private var view: CloudBackupVerificationView {
        section.view
    }

    private var undecryptableWalletCount: UInt32 {
        guard case let .needsAttention(report: report, checkedAt: _) = view.state else { return 0 }
        return report.walletIssues.decryptionFailed
    }

    var body: some View {
        switch view.state {
        case .notVerified, .required:
            CloudBackupVerificationStartSection(controls: controls(verify: verifyNow))

        // the progress section draws a running verification
        case .running:
            EmptyView()

        case let .verified(report: report, lastVerifiedAt: _):
            if let report {
                CloudBackupVerifiedSection(report: report, controls: controls(verify: verifyAgain))
            } else {
                CloudBackupPasskeyConfirmedSection(controls: controls(verify: runFullVerification))
            }

        case let .needsAttention(report: report, checkedAt: _):
            CloudBackupNeedsAttentionSection(report: report, controls: controls(verify: verifyAgain))

        case .awaitingUploadConfirmation:
            CloudBackupPendingUploadConfirmationSection(
                syncState: wallets?.sync,
                controls: controls(verify: verifyNow)
            )
            CloudBackupUploadConfirmationPendingSection()

        case .notCompleted:
            CloudBackupVerificationNotCompletedSection(controls: controls(verify: verifyNow))

        case let .failed(failure):
            CloudBackupVerificationFailureSection(
                failure: failure,
                controls: controls(verify: retry(failure))
            )
        }
    }

    private var verifyNow: CloudBackupVerifyButton {
        .init(title: "Verify Now", systemImage: "checkmark.shield", perform: startVerification)
    }

    private var verifyAgain: CloudBackupVerifyButton {
        .init(title: "Verify Again", systemImage: "checkmark.shield", perform: startVerification)
    }

    private var runFullVerification: CloudBackupVerifyButton {
        .init(title: "Run Full Verification", systemImage: "checkmark.shield", perform: startVerification)
    }

    /// A failure's Verify is a Try Again, through the discoverable passkey when the failure asks
    private func retry(_ failure: DeepVerificationFailure) -> CloudBackupVerifyButton {
        var retryAction: CloudBackupRetryAction?
        if case let .retry(_, _, action) = failure { retryAction = action }

        return .init(title: "Try Again", systemImage: "arrow.clockwise") {
            if retryAction == .verifyDiscoverable {
                manager.dispatch(action: .startVerificationDiscoverable(.cloudBackupDetail))
            } else {
                startVerification()
            }
        }
    }

    private func controls(verify: CloudBackupVerifyButton) -> CloudBackupVerificationControls {
        CloudBackupVerificationControls(
            controls: section.controls,
            actions: actions,
            verify: verify,
            recovery: view.recovery,
            undecryptableCount: undecryptableWalletCount,
            onConfirmRecovery: confirmDestructive,
            onDeleteUndecryptable: requestUndecryptableWalletDeletion,
            onRetryUpload: { manager.dispatch(action: .syncUnsynced) }
        )
    }

    private func startVerification() {
        manager.dispatch(action: .startVerification(.cloudBackupDetail))
    }

    private func confirmDestructive(_ confirmation: CloudBackupDestructiveConfirmation) {
        presentationCoordinator.present(.dialog(.destructive(confirmation)))
    }

    private func requestUndecryptableWalletDeletion(_ consent: CloudBackupBatchDeleteConsent) {
        presentationCoordinator.present(
            .alert(.undecryptableWalletDeletion(undecryptableWalletCount, consent))
        )
    }
}

/// How the current state titles its Verify control, and what it runs
struct CloudBackupVerifyButton {
    let title: String
    let systemImage: String
    let perform: () -> Void
}

/// The verification section's controls, in the order Rust names them
struct CloudBackupVerificationControls: View {
    let controls: [CloudBackupVerificationControl]
    let actions: CloudBackupActions
    let verify: CloudBackupVerifyButton
    let recovery: CloudBackupRecoveryOperation
    let undecryptableCount: UInt32
    let onConfirmRecovery: (CloudBackupDestructiveConfirmation) -> Void
    let onDeleteUndecryptable: (CloudBackupBatchDeleteConsent) -> Void
    let onRetryUpload: () -> Void

    var body: some View {
        ForEach(controls, id: \.self) { control in
            switch control {
            case .verify:
                CloudBackupActionControl(action: .verify, actions: actions, perform: verify.perform) {
                    ProbedLabel(verify.title, systemImage: verify.systemImage)
                }

            case let .recover(kind):
                CloudBackupRecoverControl(
                    confirmation: kind.confirmation,
                    recovery: recovery,
                    actions: actions,
                    onConfirm: onConfirmRecovery
                )

            case let .deleteUnreadableBackups(consent):
                CloudBackupUndecryptableWalletIssue(
                    count: undecryptableCount,
                    actions: actions,
                    onDelete: { onDeleteUndecryptable(consent) }
                )

            case .retryUpload:
                CloudBackupActionControl(action: .syncUnsynced, actions: actions, perform: onRetryUpload) {
                    Label("Try Again", systemImage: "arrow.clockwise")
                }

            // only Google Drive asks to be authorized again; iCloud never does
            case .reconnectDrive:
                EmptyView()
            }
        }
    }
}

extension CloudBackupRecoveryKind {
    /// The confirmation this rebuild asks for before it runs
    var confirmation: CloudBackupDestructiveConfirmation {
        switch self {
        case .recreateManifest: .recreate
        case .reinitializeBackup: .reinitialize
        }
    }
}

private struct CloudBackupVerificationStartSection: View {
    let controls: CloudBackupVerificationControls

    var body: some View {
        Section {
            Text("Run verification to confirm your cloud backup can be decrypted and restored")
                .font(.caption)
                .uiTestTruncationProbe()
                .foregroundStyle(.secondary)

            controls
        }
    }
}

/// A verification the passkey prompt didn't finish, such as a cancel; it never offers a passkey
/// repair, since nothing showed the passkey is gone, so Verify is the way forward
private struct CloudBackupVerificationNotCompletedSection: View {
    let controls: CloudBackupVerificationControls

    var body: some View {
        Section {
            ProbedLabel(
                "Cloud Backup Not Verified",
                systemImage: "exclamationmark.shield.fill"
            )
            .foregroundStyle(Color.statusWarning)

            Text("Verification didn't finish. Verify again to confirm your cloud backup can be restored.")
                .font(.caption)
                .uiTestTruncationProbe()
                .foregroundStyle(.secondary)

            controls
        }
    }
}

private struct CloudBackupVerifiedSection: View {
    let report: DeepVerificationReport
    let controls: CloudBackupVerificationControls

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
            ProbedLabel("Backup verified", systemImage: "checkmark.shield.fill")
                .foregroundStyle(Color.statusSuccess)
                .alignmentGuide(.listRowSeparatorLeading) { _ in 0 }

            if let summary {
                Text(summary)
                    .font(.caption)
                    .uiTestTruncationProbe()
                    .foregroundStyle(.secondary)
            }
        }

        Section {
            controls
        }
    }
}

/// A report with wallet issues; the undecryptable backups' delete, when Rust names it, is drawn
/// with Verify Again under the issues
private struct CloudBackupNeedsAttentionSection: View {
    let report: DeepVerificationReport
    let controls: CloudBackupVerificationControls

    var body: some View {
        Section {
            ProbedLabel("Backup needs attention", systemImage: "exclamationmark.shield.fill")
                .foregroundStyle(Color.statusWarning)
                .alignmentGuide(.listRowSeparatorLeading) { _ in 0 }

            Text("The backup key is valid. \(report.walletsVerified) wallet backup(s) passed verification.")
                .font(.caption)
                .uiTestTruncationProbe()
                .foregroundStyle(.secondary)

            CloudBackupWalletIssueRows(issues: report.walletIssues)
        }

        Section {
            controls
        }
    }
}

/// The report's wallet issues; undecryptable backups are listed by their delete control
private struct CloudBackupWalletIssueRows: View {
    let issues: CloudBackupWalletVerificationIssues

    var body: some View {
        CloudBackupWalletIssueLabel(
            count: issues.missing,
            message: "wallet backup file(s) are missing from cloud storage"
        )
        CloudBackupWalletIssueLabel(
            count: issues.downloadFailed,
            message: "wallet backup(s) could not be downloaded"
        )
        CloudBackupWalletIssueLabel(
            count: issues.invalid,
            message: "wallet backup file(s) contain invalid data"
        )
        CloudBackupWalletIssueLabel(
            count: issues.unsupported,
            message: "wallet backup(s) use a newer backup format"
        )
        CloudBackupWalletIssueLabel(
            count: issues.unreadable,
            message: "wallet backup(s) could not be read"
        )
    }
}

private struct CloudBackupWalletIssueLabel: View {
    let count: UInt32
    let message: String

    var body: some View {
        if count > 0 {
            ProbedLabel("\(count) \(message)", systemImage: "exclamationmark.triangle.fill")
                .foregroundStyle(Color.statusError)
                .font(.caption)
        }
    }
}

private struct CloudBackupUndecryptableWalletIssue: View {
    let count: UInt32
    let actions: CloudBackupActions
    let onDelete: () -> Void

    var body: some View {
        CloudBackupActionControl(
            action: .deleteUnreadableBackups,
            actions: actions,
            role: .destructive,
            perform: onDelete
        ) {
            HStack {
                Image(systemName: "exclamationmark.triangle.fill")

                Text("\(count) wallet backup(s) could not be decrypted with this backup key")
                    .uiTestTruncationProbe()
                Spacer()
                Image(systemName: "chevron.right")
                    .font(.caption)
            }
        }
        .font(.caption)
        .accessibilityHint("Opens a confirmation to delete these inaccessible backups")
    }
}

/// A failed verification: its message, and the one recovery Rust names for it, if any
private struct CloudBackupVerificationFailureSection: View {
    let failure: DeepVerificationFailure
    let controls: CloudBackupVerificationControls

    var body: some View {
        Section {
            switch failure {
            case let .retry(message, _, _):
                ProbedLabel(message, systemImage: "exclamationmark.triangle.fill")
                    .foregroundStyle(Color.statusWarning)

            case let .recreateManifest(message, warning, _),
                 let .reinitializeBackup(message, warning, _):
                ProbedLabel(message, systemImage: "exclamationmark.triangle.fill")
                    .foregroundStyle(Color.statusError)

                Text(warning)
                    .font(.caption)
                    .uiTestTruncationProbe()
                    .foregroundStyle(.secondary)

            case let .unsupportedVersion(message, _):
                CloudBackupUnsupportedVersionFailureContent(message: message)
            }

            controls
        }
    }
}

/// A recreate or a reinitialize, which asks to confirm before it runs
private struct CloudBackupRecoverControl: View {
    let confirmation: CloudBackupDestructiveConfirmation
    let recovery: CloudBackupRecoveryOperation
    let actions: CloudBackupActions
    let onConfirm: (CloudBackupDestructiveConfirmation) -> Void

    private var isRunning: Bool {
        switch confirmation {
        case .recreate: recovery == .recreatingManifest
        case .reinitialize: recovery == .reinitializingBackup
        }
    }

    private var title: String {
        switch confirmation {
        case .recreate: "Recreate Backup Index"
        case .reinitialize: "Reinitialize Cloud Backup"
        }
    }

    private var progressTitle: String {
        switch confirmation {
        case .recreate: "Recreating..."
        case .reinitialize: "Reinitializing..."
        }
    }

    private var systemImage: String {
        switch confirmation {
        case .recreate: "arrow.clockwise"
        case .reinitialize: "arrow.counterclockwise"
        }
    }

    var body: some View {
        CloudBackupActionControl(
            action: confirmation.action,
            actions: actions,
            role: .destructive,
            perform: { onConfirm(confirmation) }
        ) {
            if isRunning {
                HStack {
                    ProgressView()
                        .padding(.trailing, 4)
                    Text(progressTitle)
                        .uiTestTruncationProbe()
                }
            } else {
                ProbedLabel(title, systemImage: systemImage)
            }
        }
    }
}

private struct CloudBackupUnsupportedVersionFailureContent: View {
    let message: String

    var body: some View {
        ProbedLabel(message, systemImage: "exclamationmark.triangle.fill")
            .foregroundStyle(Color.statusWarning)

        Text("Please update the app to the latest version")
            .font(.caption)
            .uiTestTruncationProbe()
            .foregroundStyle(.secondary)
    }
}

private struct CloudBackupUploadConfirmationPendingSection: View {
    var body: some View {
        Section {
            ProbedLabel("Cloud Backup enabled", systemImage: "icloud.and.arrow.up.fill")
                .foregroundStyle(Color.statusSuccess)

            Text("Cove is still confirming that your encrypted backup is visible in iCloud. You can leave this screen while confirmation continues in the background.")
                .font(.caption)
                .uiTestTruncationProbe()
                .foregroundStyle(.secondary)
        }
    }
}

private struct CloudBackupPasskeyConfirmedSection: View {
    let controls: CloudBackupVerificationControls

    var body: some View {
        Section {
            ProbedLabel("Passkey verified", systemImage: "checkmark.shield.fill")
                .foregroundStyle(Color.statusSuccess)

            Text("Your stored passkey is valid. Run a full verification to confirm wallet backups can be decrypted.")
                .font(.caption)
                .uiTestTruncationProbe()
                .foregroundStyle(.secondary)

            controls
        }
    }
}

/// Sync Now for the wallets waiting to upload, with the sync's progress or failure
struct CloudBackupSyncNowControl: View {
    let syncState: CloudBackupSyncState?
    let actions: CloudBackupActions
    let onSync: () -> Void

    var body: some View {
        CloudBackupActionControl(action: .syncUnsynced, actions: actions, perform: onSync) {
            HStack {
                if case .syncing = syncState {
                    ProgressView()
                        .padding(.trailing, 8)
                    Text("Syncing...")
                        .uiTestTruncationProbe()
                } else {
                    Image(systemName: "arrow.triangle.2.circlepath")
                    Text("Sync Now")
                        .uiTestTruncationProbe()
                }
            }
        }

        if case let .failed(error) = syncState {
            Text(error)
                .font(.caption)
                .uiTestTruncationProbe()
                .foregroundStyle(Color.statusError)
        }
    }
}
