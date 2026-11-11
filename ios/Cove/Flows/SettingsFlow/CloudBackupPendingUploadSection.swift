import SwiftUI

enum CloudBackupPendingUploadAccessibilityStatus: Hashable {
    case confirming
    case authorizationRequired
    case failed
}

/// The status of an upload Cove is still confirming, by what blocks the sync
func cloudBackupPendingUploadAccessibilityStatus(
    syncState: CloudBackupSyncState?
) -> CloudBackupPendingUploadAccessibilityStatus {
    switch syncState {
    case .blocked: .authorizationRequired
    case .failed: .failed
    default: .confirming
    }
}

/// Drawn while verification waits for the latest upload to be confirmed, with the controls the
/// verification section names for it
struct CloudBackupPendingUploadConfirmationSection: View {
    let syncState: CloudBackupSyncState?
    let controls: CloudBackupVerificationControls

    @AccessibilityFocusState private var focusedStatus: CloudBackupPendingUploadAccessibilityStatus?

    private var accessibilityStatus: CloudBackupPendingUploadAccessibilityStatus {
        cloudBackupPendingUploadAccessibilityStatus(syncState: syncState)
    }

    var body: some View {
        Group {
            switch accessibilityStatus {
            case .confirming:
                CloudBackupUploadConfirmingSection()
            case .authorizationRequired:
                CloudBackupUploadAuthorizationRequiredSection(focusedStatus: $focusedStatus)
            case .failed:
                if case let .failed(message) = syncState {
                    CloudBackupUploadConfirmationFailedSection(
                        controls: controls,
                        message: message,
                        focusedStatus: $focusedStatus
                    )
                }
            }
        }
        .onChange(of: accessibilityStatus, initial: true) { _, status in
            switch status {
            case .authorizationRequired, .failed:
                focusedStatus = status
            case .confirming:
                focusedStatus = nil
            }
        }
    }
}

private struct CloudBackupUploadConfirmingSection: View {
    var body: some View {
        Section {
            HStack {
                ProgressView()
                    .padding(.trailing, 8)

                Text("Confirming latest cloud upload")
            }
        }
    }
}

private struct CloudBackupUploadAuthorizationRequiredSection: View {
    @AccessibilityFocusState.Binding var focusedStatus: CloudBackupPendingUploadAccessibilityStatus?

    var body: some View {
        Section {
            Label("Waiting for iCloud authorization", systemImage: "icloud.slash")
                .foregroundStyle(.orange)
                .accessibilityFocused($focusedStatus, equals: .authorizationRequired)
        }
    }
}

private struct CloudBackupUploadConfirmationFailedSection: View {
    let controls: CloudBackupVerificationControls
    let message: String
    @AccessibilityFocusState.Binding var focusedStatus: CloudBackupPendingUploadAccessibilityStatus?

    var body: some View {
        Section {
            Label("Latest upload could not be confirmed", systemImage: "exclamationmark.icloud")
                .foregroundStyle(Color.statusError)
                .accessibilityFocused($focusedStatus, equals: .failed)

            Text(message)
                .font(.caption)
                .foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)

            controls
        }
    }
}
