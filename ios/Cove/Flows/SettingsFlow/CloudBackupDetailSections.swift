import SwiftUI

/// The passkey card above the list, for a passkey Cloud Backup can't use
struct CloudBackupPasskeyCard: View {
    let state: CloudBackupPasskeyState
    let actions: CloudBackupActions
    let manager: CloudBackupManager

    var body: some View {
        switch state {
        case .available:
            EmptyView()
        case .missing:
            MissingPasskeyContent(repairState: nil, actions: actions, manager: manager)
        case let .needsRepair(repairState):
            MissingPasskeyContent(repairState: repairState, actions: actions, manager: manager)
        case .unsupportedProvider:
            UnsupportedPasskeyProviderContent(actions: actions, manager: manager)
        }
    }
}

private struct MissingPasskeyContent: View {
    let repairState: CloudBackupPasskeyRepairState?
    let actions: CloudBackupActions
    let manager: CloudBackupManager

    private var isRepairing: Bool {
        if case .running = repairState { return true }
        return false
    }

    private var repairError: String? {
        guard case let .failed(error) = repairState else { return nil }
        return error
    }

    var body: some View {
        Section {
            VStack(spacing: 12) {
                Image(systemName: "exclamationmark.icloud.fill")
                    .font(.system(size: 36))
                    .foregroundStyle(Color.statusError)

                Text("Cloud Backup Passkey Missing")
                    .font(.headline)
                    .uiTestTruncationProbe()
                    .foregroundStyle(Color.statusError)

                Text(
                    "Your cloud backup is not accessible until you use an existing passkey or add a new one. Without it, your backups can't be restored."
                )
                .font(.subheadline)
                .uiTestTruncationProbe()
                .foregroundStyle(Color.statusError.opacity(0.85))
                .multilineTextAlignment(.center)
            }
            .frame(maxWidth: .infinity)
            .padding(.vertical, 12)
        }

        Section {
            CloudBackupActionControl(
                action: .repairPasskey,
                actions: actions,
                perform: { manager.dispatch(action: .repairPasskeyNoDiscovery) }
            ) {
                if isRepairing {
                    HStack {
                        ProgressView()
                            .padding(.trailing, 4)
                        Text("Opening Passkey Options...")
                            .uiTestTruncationProbe()
                    }
                } else {
                    ProbedLabel("Add Passkey", systemImage: "person.badge.key")
                }
            }

            Text("Use an existing passkey or add a new one to make your cloud backup accessible again")
                .font(.caption)
                .uiTestTruncationProbe()
                .foregroundStyle(.secondary)

            if let repairError {
                ProbedLabel(repairError, systemImage: "exclamationmark.triangle.fill")
                    .foregroundStyle(Color.statusError)
                    .font(.caption)
            }
        }
    }
}

private struct UnsupportedPasskeyProviderContent: View {
    let actions: CloudBackupActions
    let manager: CloudBackupManager

    var body: some View {
        Section {
            VStack(spacing: 12) {
                Image(systemName: "exclamationmark.shield.fill")
                    .font(.system(size: 36))
                    .foregroundStyle(.red)

                Text("Passkey Not Supported for Cloud Backup")
                    .font(.headline)
                    .foregroundStyle(.red)

                Text(
                    "This passkey provider can't create the secure passkey required for Cloud Backup."
                )
                .font(.subheadline)
                .foregroundStyle(.red.opacity(0.85))
                .multilineTextAlignment(.center)

                Text(
                    "Try again with a supported password manager on iOS such as Apple Passwords, 1Password, or Bitwarden."
                )
                .font(.caption)
                .foregroundStyle(.secondary)
                .multilineTextAlignment(.center)
            }
            .frame(maxWidth: .infinity)
            .padding(.vertical, 12)
        }

        Section {
            // a configured backup gets a new passkey through repair; Enable is for a backup that is off
            CloudBackupActionControl(
                action: .repairPasskey,
                actions: actions,
                perform: { manager.dispatch(action: .repairPasskeyNoDiscovery) }
            ) {
                Label("Try Again", systemImage: "arrow.clockwise")
            }
        }
    }
}

/// Turn Off, with the progress of a Turn Off Rust is preparing or draining
///
/// The review and the offline case are dialogs, drawn by the detail screen's presentations
struct CloudBackupTurnOffSection: View {
    let disable: CloudBackupDisableScreen
    let actions: CloudBackupActions
    let manager: CloudBackupManager

    private var progressTitle: String? {
        switch disable {
        case .preparing: "Checking what's in Cloud Backup..."
        case .draining: "Turning off Cloud Backup..."
        case .idle, .reviewing, .offline: nil
        }
    }

    var body: some View {
        Section {
            if let progressTitle {
                HStack {
                    ProgressView()
                        .padding(.trailing, 8)
                    Text(progressTitle)
                        .font(.footnote)
                        .uiTestTruncationProbe()
                }
                .accessibilityIdentifier("cloudBackup.turnOff.progress")

                CloudBackupActionControl(
                    action: .cancelDisable,
                    actions: actions,
                    perform: { manager.dispatch(action: .cancelDisable) }
                ) {
                    Text("Cancel").font(.footnote).uiTestTruncationProbe()
                }
            }

            CloudBackupActionControl(
                action: .turnOff,
                actions: actions,
                role: .destructive,
                perform: { manager.dispatch(action: .disableCloudBackup) }
            ) {
                Text("Turn Off Cloud Backup")
                    .font(.footnote)
                    .uiTestTruncationProbe()
            }
            .alignmentGuide(.listRowSeparatorLeading) { _ in 0 }
        }
    }
}
