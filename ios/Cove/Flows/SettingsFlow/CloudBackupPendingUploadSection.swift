import SwiftUI

enum CloudBackupPendingUploadAccessibilityStatus: Hashable {
    case hidden
    case confirming
    case authorizationRequired
    case failed
}

func cloudBackupPendingUploadAccessibilityStatus(
    verificationState: CloudBackupVerificationState?,
    syncState: CloudBackupSyncState?
) -> CloudBackupPendingUploadAccessibilityStatus {
    guard case .awaitingUploadConfirmation = verificationState else { return .hidden }

    return switch syncState {
    case .blocked: .authorizationRequired
    case .failed: .failed
    default: .confirming
    }
}

struct CloudBackupPendingUploadConfirmationSection: View {
    let manager: CloudBackupManager

    @AccessibilityFocusState private var focusedStatus: CloudBackupPendingUploadAccessibilityStatus?

    private var accessibilityStatus: CloudBackupPendingUploadAccessibilityStatus {
        cloudBackupPendingUploadAccessibilityStatus(
            verificationState: manager.verificationState,
            syncState: manager.syncState
        )
    }

    var body: some View {
        Group {
            switch accessibilityStatus {
            case .hidden:
                EmptyView()
            case .confirming:
                CloudBackupUploadConfirmingSection()
            case .authorizationRequired:
                CloudBackupUploadAuthorizationRequiredSection(focusedStatus: $focusedStatus)
            case .failed:
                if case let .failed(message) = manager.syncState {
                    CloudBackupUploadConfirmationFailedSection(
                        manager: manager,
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
            case .hidden, .confirming:
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
    let manager: CloudBackupManager
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

            Button(action: syncUnsynced) {
                Label("Try Again", systemImage: "arrow.clockwise")
            }
        }
    }

    private func syncUnsynced() {
        manager.dispatch(action: .syncUnsynced)
    }
}
