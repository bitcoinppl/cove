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
        case .verified, .awaitingUploadConfirmation, .failed: true
        default: false
        }
    }

    private var isCancelled: Bool {
        false
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
                onRecreate: { showRecreateConfirmation = true },
                onReinitialize: { showReinitializeConfirmation = true }
            )
        }
        .navigationTitle("Cloud Backup")
        .navigationBarTitleDisplayMode(.inline)
        .task {
            manager.dispatch(action: .enterDetail)
        }
        .onDisappear {
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
                manager.dispatch(action: .recreateManifest)
            }
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
                manager.dispatch(action: .reinitializeBackup)
            }
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
            DisableCloudBackupSection(manager: manager, detail: manager.detail)
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
            if manager.detail != nil {
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

    var body: some View {
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
        } else if shouldShowLoadingState {
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
}

struct CloudBackupPendingUploadConfirmationSection: View {
    let manager: CloudBackupManager

    var body: some View {
        switch manager.verificationState {
        case .awaitingUploadConfirmation:
            if case .blocked = manager.syncState {
                Section {
                    Label("Waiting for iCloud authorization", systemImage: "icloud.slash")
                        .foregroundStyle(.orange)
                }
            } else if case let .failed(message) = manager.syncState {
                Section {
                    Label("Latest upload could not be confirmed", systemImage: "exclamationmark.icloud")
                        .foregroundStyle(Color.statusError)

                    Text(message)
                        .font(.caption)
                        .foregroundStyle(.secondary)

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
