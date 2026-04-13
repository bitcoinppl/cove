import SwiftUI

struct CloudBackupDetailScreen: View {
    @Environment(CloudBackupPresentationCoordinator.self)
    private var cloudBackupPresentationCoordinator
    @State private var manager = CloudBackupManager.shared
    @State private var syncHealth: ICloudDriveHelper.SyncHealth = .noFiles
    @State private var showRecreateConfirmation = false
    @State private var showReinitializeConfirmation = false
    @State private var hasAutoVerified = false

    private var isVerifying: Bool {
        if case .verifying = manager.verification { return true }
        return false
    }

    private var hasVerificationResult: Bool {
        switch manager.verification {
        case .verified, .passkeyConfirmed, .failed, .cancelled: true
        default: false
        }
    }

    private var isCancelled: Bool {
        if case .cancelled = manager.verification { return true }
        return false
    }

    private var isPasskeyMissing: Bool {
        if case .passkeyMissing = manager.status { return true }
        return false
    }

    private var isUnsupportedPasskeyProvider: Bool {
        if case .unsupportedPasskeyProvider = manager.status { return true }
        return false
    }

    private var shouldShowLoadingState: Bool {
        manager.detail == nil && !isVerifying && !hasVerificationResult && !isCancelled
    }

    private var hasCloudBackupPresentationBlocker: Bool {
        showRecreateConfirmation || showReinitializeConfirmation
    }

    var body: some View {
        Form {
            formContent
        }
        .navigationTitle("Cloud Backup")
        .navigationBarTitleDisplayMode(.inline)
        .task {
            guard !isPasskeyMissing, !isUnsupportedPasskeyProvider else { return }

            refreshSyncHealth()
            manager.dispatch(action: .refreshDetail)

            if !hasAutoVerified {
                hasAutoVerified = true
                manager.dispatch(action: .startVerificationDiscoverable)
            }
        }
        .onDisappear {
            cloudBackupPresentationCoordinator.setBlocker(.cloudBackupDetailDialog, active: false)
        }
        .onChange(of: manager.detail) { _, _ in
            refreshSyncHealth()
        }
        .onChange(of: manager.verification) { _, _ in
            refreshSyncHealth()
        }
        .onChange(of: hasCloudBackupPresentationBlocker, initial: true) { _, active in
            cloudBackupPresentationCoordinator.setBlocker(.cloudBackupDetailDialog, active: active)
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

    private func refreshSyncHealth() {
        syncHealth = ICloudDriveHelper.shared.overallSyncHealth()
    }

    @ViewBuilder
    private var formContent: some View {
        if isUnsupportedPasskeyProvider {
            UnsupportedPasskeyProviderContent(manager: manager)
        } else if isPasskeyMissing {
            MissingPasskeyContent(manager: manager)
        } else {
            backupStatusContent
            VerificationSection(
                manager: manager,
                onRecreate: { showRecreateConfirmation = true },
                onReinitialize: { showReinitializeConfirmation = true }
            )
        }
    }

    @ViewBuilder
    private var backupStatusContent: some View {
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
                syncHealth: syncHealth,
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
                manager.dispatch(action: .enableCloudBackupNoDiscovery)
            } label: {
                Label("Try Again", systemImage: "arrow.clockwise")
            }

            Button("Back", role: .cancel) {
                dismiss()
            }
        }
    }
}
