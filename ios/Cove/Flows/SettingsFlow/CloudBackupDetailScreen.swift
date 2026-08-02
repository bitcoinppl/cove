import SwiftUI

private enum CloudBackupDestructiveConfirmation: Equatable {
    case recreate
    case reinitialize
}

struct CloudBackupDetailScreen: View {
    @Environment(\.dismiss) private var dismiss
    @Environment(CloudBackupPresentationCoordinator.self)
    private var cloudBackupPresentationCoordinator
    @State private var manager = CloudBackupManager.shared
    @State private var destructiveConfirmation: CloudBackupDestructiveConfirmation?

    private var isVerifying: Bool {
        if case .running = manager.verificationState { return true }
        return false
    }

    private var hasVerificationResult: Bool {
        switch manager.verificationState {
        case .verified, .awaitingUploadConfirmation, .cancelled, .failed: true
        default: false
        }
    }

    private var isCancelled: Bool {
        if case .cancelled = manager.verificationState {
            return true
        }
        return false
    }

    private var isPasskeyMissing: Bool {
        manager.isPasskeyMissing
    }

    private var isUnsupportedPasskeyProvider: Bool {
        manager.isUnsupportedPasskeyProvider
    }

    private var shouldShowLoadingState: Bool {
        manager.detail == nil && !isVerifying && !hasVerificationResult && !isCancelled
    }

    private var hasCloudBackupPresentationBlocker: Bool {
        destructiveConfirmation != nil
    }

    var body: some View {
        CloudBackupDetailForm(
            manager: manager,
            isVerifying: isVerifying,
            hasVerificationResult: hasVerificationResult,
            isCancelled: isCancelled,
            isPasskeyMissing: isPasskeyMissing,
            isUnsupportedPasskeyProvider: isUnsupportedPasskeyProvider,
            shouldShowLoadingState: shouldShowLoadingState,
            recreateConfirmationIsPresented: confirmationBinding(for: .recreate),
            reinitializeConfirmationIsPresented: confirmationBinding(for: .reinitialize)
        )
        .navigationTitle("Cloud Backup")
        .navigationBarTitleDisplayMode(.inline)
        .task(enterDetail)
        .onDisappear(perform: closeDetail)
        .onChange(of: hasCloudBackupPresentationBlocker, initial: true) { _, active in
            cloudBackupPresentationCoordinator.setBlocker(.cloudBackupDetailDialog, active: active)
        }
        .onChange(of: manager.isLifecycleDisabled) { _, isDisabled in
            if isDisabled {
                dismiss()
            }
        }
    }

    private func confirmationBinding(
        for confirmation: CloudBackupDestructiveConfirmation
    ) -> Binding<Bool> {
        Binding(
            get: { destructiveConfirmation == confirmation },
            set: { presented in
                if presented {
                    guard manager.isDetailInventoryComplete else { return }

                    destructiveConfirmation = confirmation
                } else if destructiveConfirmation == confirmation {
                    destructiveConfirmation = nil
                }
            }
        )
    }

    private func enterDetail() async {
        manager.dispatch(action: .enterDetail)
    }

    private func closeDetail() {
        manager.dispatch(action: .closeDetail)
        cloudBackupPresentationCoordinator.setBlocker(.cloudBackupDetailDialog, active: false)
    }
}

private struct CloudBackupDetailForm: View {
    let manager: CloudBackupManager
    let isVerifying: Bool
    let hasVerificationResult: Bool
    let isCancelled: Bool
    let isPasskeyMissing: Bool
    let isUnsupportedPasskeyProvider: Bool
    let shouldShowLoadingState: Bool
    let recreateConfirmationIsPresented: Binding<Bool>
    let reinitializeConfirmationIsPresented: Binding<Bool>

    var body: some View {
        Form {
            CloudBackupDetailFormContent(
                manager: manager,
                isVerifying: isVerifying,
                hasVerificationResult: hasVerificationResult,
                isCancelled: isCancelled,
                isPasskeyMissing: isPasskeyMissing,
                isUnsupportedPasskeyProvider: isUnsupportedPasskeyProvider,
                shouldShowLoadingState: shouldShowLoadingState,
                recreateConfirmationIsPresented: recreateConfirmationIsPresented,
                reinitializeConfirmationIsPresented: reinitializeConfirmationIsPresented
            )
        }
    }
}

struct CloudBackupDetailFormContent: View {
    let manager: CloudBackupManager
    let isVerifying: Bool
    let hasVerificationResult: Bool
    let isCancelled: Bool
    let isPasskeyMissing: Bool
    let isUnsupportedPasskeyProvider: Bool
    let shouldShowLoadingState: Bool
    let recreateConfirmationIsPresented: Binding<Bool>
    let reinitializeConfirmationIsPresented: Binding<Bool>

    var body: some View {
        if isUnsupportedPasskeyProvider {
            UnsupportedPasskeyProviderContent(manager: manager)
        } else if isPasskeyMissing {
            MissingPasskeyContent(manager: manager)

            if manager.isDetailInventoryComplete {
                DisableCloudBackupSection(manager: manager, detail: manager.detail)
            }
        } else {
            CloudBackupPendingUploadConfirmationSection(manager: manager)

            CloudBackupStatusSection(
                manager: manager,
                isVerifying: isVerifying,
                hasVerificationResult: hasVerificationResult,
                isCancelled: isCancelled,
                shouldShowLoadingState: shouldShowLoadingState
            )
            VerificationSection(
                manager: manager,
                recreateConfirmationIsPresented: recreateConfirmationIsPresented,
                reinitializeConfirmationIsPresented: reinitializeConfirmationIsPresented
            )
            if manager.detail != nil, manager.isDetailInventoryComplete {
                DisableCloudBackupSection(manager: manager, detail: manager.detail)
            }
        }
    }
}

struct CloudBackupStatusSection: View {
    let manager: CloudBackupManager
    let isVerifying: Bool
    let hasVerificationResult: Bool
    let isCancelled: Bool
    let shouldShowLoadingState: Bool

    @AccessibilityFocusState private var inventoryErrorFocused: Bool

    var body: some View {
        Group {
            if manager.isDetailInventoryChecking, manager.detail != nil {
                CloudBackupInventoryCheckingSection()
            }

            if let error = manager.detailError {
                CloudBackupInventoryErrorSection(
                    manager: manager,
                    error: error,
                    isFocused: $inventoryErrorFocused
                )
            }

            CloudBackupDetailStatusContent(
                manager: manager,
                isVerifying: isVerifying,
                hasVerificationResult: hasVerificationResult,
                isCancelled: isCancelled,
                shouldShowLoadingState: shouldShowLoadingState
            )
        }
        .onChange(of: manager.detailError, initial: true) { _, error in
            inventoryErrorFocused = error != nil
        }
    }
}

private struct CloudBackupInventoryCheckingSection: View {
    var body: some View {
        Section {
            HStack {
                ProgressView()
                    .padding(.trailing, 8)

                Text("Checking for more cloud backups...")
            }
            .foregroundStyle(.secondary)
            .accessibilityIdentifier("cloudBackup.inventory.checking")
        }
    }
}

private struct CloudBackupInventoryErrorSection: View {
    let manager: CloudBackupManager
    let error: String
    @AccessibilityFocusState.Binding var isFocused: Bool

    var body: some View {
        Section {
            Label("Cloud backup inventory is incomplete", systemImage: "exclamationmark.icloud")
                .foregroundStyle(Color.statusError)
                .accessibilityFocused($isFocused)
                .accessibilityIdentifier("cloudBackup.inventory.incomplete")

            Text(error)
                .font(.caption)
                .foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)

            Button(action: refreshDetail) {
                Label("Check Again", systemImage: "arrow.clockwise")
            }
            .accessibilityIdentifier("cloudBackup.inventory.checkAgain")
        }
    }

    private func refreshDetail() {
        manager.dispatch(action: .refreshDetail)
    }
}

private struct CloudBackupDetailStatusContent: View {
    let manager: CloudBackupManager
    let isVerifying: Bool
    let hasVerificationResult: Bool
    let isCancelled: Bool
    let shouldShowLoadingState: Bool

    var body: some View {
        if isVerifying, !hasVerificationResult {
            CloudBackupVerifyingSection()
        } else if let detail = manager.detail, !isCancelled {
            DetailFormContent(
                detail: detail,
                syncHealth: manager.syncHealth,
                manager: manager
            )
        } else if shouldShowLoadingState, manager.detailError == nil {
            CloudBackupLoadingSection()
        }
    }
}

private struct CloudBackupVerifyingSection: View {
    var body: some View {
        Section {
            VStack {
                ProgressView("Verifying cloud backup...")
            }
            .frame(maxWidth: .infinity)
            .padding(.vertical, 8)
        }
    }
}

private struct CloudBackupLoadingSection: View {
    var body: some View {
        Section {
            VStack(spacing: 12) {
                ProgressView("Loading cloud backup...")

                Text("Finishing setup and fetching backup details")
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .multilineTextAlignment(.center)
            }
            .frame(maxWidth: .infinity)
            .padding(.vertical, 8)
        }
    }
}

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

struct UnsupportedPasskeyProviderContent: View {
    @Environment(\.dismiss) private var dismiss
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
                    "This passkey provider can't create the secure passkey required for Cloud Backup. No cloud backup was enabled from this attempt."
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
            Button {
                manager.dispatch(action: .enableCloudBackupNoDiscovery(.init(
                    savedPasskeyConfirmation: .manual,
                    verificationSource: .cloudBackupDetail
                )))
            } label: {
                Label("Try Again", systemImage: "arrow.clockwise")
            }

            Button("Back", role: .cancel) {
                dismiss()
            }
        }
    }
}
