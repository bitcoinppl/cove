import SwiftUI

struct CloudBackupDetailScreen: View {
    @Environment(\.dismiss) private var dismiss
    @Environment(CloudBackupPresentationCoordinator.self)
    private var cloudBackupPresentationCoordinator
    @State private var manager = CloudBackupManager.shared
    @State private var showRecreateConfirmation = false
    @State private var showReinitializeConfirmation = false

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
        showRecreateConfirmation || showReinitializeConfirmation
    }

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
                onRecreate: {
                    guard manager.isDetailInventoryComplete else { return }

                    showRecreateConfirmation = true
                },
                onReinitialize: {
                    guard manager.isDetailInventoryComplete else { return }

                    showReinitializeConfirmation = true
                }
            )
        }
        .navigationTitle("Cloud Backup")
        .navigationBarTitleDisplayMode(.inline)
        .task {
            manager.dispatch(action: .enterDetail)
        }
        .onDisappear {
            manager.dispatch(action: .closeDetail)
            cloudBackupPresentationCoordinator.setBlocker(.cloudBackupDetailDialog, active: false)
        }
        .onChange(of: hasCloudBackupPresentationBlocker, initial: true) { _, active in
            cloudBackupPresentationCoordinator.setBlocker(.cloudBackupDetailDialog, active: active)
        }
        .onChange(of: manager.isLifecycleDisabled) { _, isDisabled in
            if isDisabled {
                dismiss()
            }
        }
        .confirmationDialog(
            "Recreate Backup Index",
            isPresented: $showRecreateConfirmation,
            titleVisibility: .visible
        ) {
            Button("Recreate", role: .destructive) {
                guard manager.isDetailInventoryComplete else { return }

                manager.dispatch(action: .recreateManifest)
            }
            .disabled(!manager.isDetailInventoryComplete)

            Button("Cancel", role: .cancel) {}
        } message: {
            Text(
                "This will rebuild the backup index from wallets on this device. Wallets that only exist in the cloud backup will no longer be referenced."
            )
        }
        .confirmationDialog(
            "Reinitialize Cloud Backup",
            isPresented: $showReinitializeConfirmation,
            titleVisibility: .visible
        ) {
            Button("Reinitialize", role: .destructive) {
                guard manager.isDetailInventoryComplete else { return }

                manager.dispatch(action: .reinitializeBackup)
            }
            .disabled(!manager.isDetailInventoryComplete)

            Button("Cancel", role: .cancel) {}
        } message: {
            Text(
                "This will replace your entire cloud backup. Wallets that only exist in the current cloud backup will be lost."
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
    let onRecreate: () -> Void
    let onReinitialize: () -> Void

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
                onRecreate: onRecreate,
                onReinitialize: onReinitialize
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

            if let error = manager.detailError {
                Section {
                    Label("Cloud backup inventory is incomplete", systemImage: "exclamationmark.icloud")
                        .foregroundStyle(Color.statusError)
                        .accessibilityFocused($inventoryErrorFocused)
                        .accessibilityIdentifier("cloudBackup.inventory.incomplete")

                    Text(error)
                        .font(.caption)
                        .foregroundStyle(.secondary)
                        .fixedSize(horizontal: false, vertical: true)

                    Button {
                        manager.dispatch(action: .refreshDetail)
                    } label: {
                        Label("Check Again", systemImage: "arrow.clockwise")
                    }
                    .accessibilityIdentifier("cloudBackup.inventory.checkAgain")
                }
            }

            if isVerifying, !hasVerificationResult {
                Section {
                    VStack {
                        ProgressView("Verifying cloud backup...")
                    }
                    .frame(maxWidth: .infinity)
                    .padding(.vertical, 8)
                }
            } else if let detail = manager.detail, !isCancelled {
                DetailFormContent(
                    detail: detail,
                    syncHealth: manager.syncHealth,
                    manager: manager
                )
            } else if shouldShowLoadingState, manager.detailError == nil {
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
        .onChange(of: manager.detailError, initial: true) { _, error in
            inventoryErrorFocused = error != nil
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
            switch manager.verificationState {
            case .awaitingUploadConfirmation:
                if case .blocked = manager.syncState {
                    Section {
                        Label("Waiting for iCloud authorization", systemImage: "icloud.slash")
                            .foregroundStyle(.orange)
                            .accessibilityFocused($focusedStatus, equals: .authorizationRequired)
                    }
                } else if case let .failed(message) = manager.syncState {
                    Section {
                        Label("Latest upload could not be confirmed", systemImage: "exclamationmark.icloud")
                            .foregroundStyle(Color.statusError)
                            .accessibilityFocused($focusedStatus, equals: .failed)

                        Text(message)
                            .font(.caption)
                            .foregroundStyle(.secondary)
                            .fixedSize(horizontal: false, vertical: true)

                        Button {
                            manager.dispatch(action: .syncUnsynced)
                        } label: {
                            Label("Try Again", systemImage: "arrow.clockwise")
                        }
                    }
                } else {
                    Section {
                        HStack {
                            ProgressView()
                                .padding(.trailing, 8)

                            Text("Confirming latest cloud upload")
                        }
                    }
                }
            default:
                EmptyView()
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
