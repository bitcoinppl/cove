import SwiftUI

struct CloudBackupDetailForm: View {
    let manager: CloudBackupManager
    let isCancelled: Bool
    let isPasskeyMissing: Bool
    let isUnsupportedPasskeyProvider: Bool
    let shouldShowLoadingState: Bool
    let progressPresentation: CloudBackupDetailProgressPresentation
    let presentationCoordinator: PresentationTransitionCoordinator<CloudBackupDetailPresentation>
    let recreateConfirmationIsPresented: Binding<Bool>
    let reinitializeConfirmationIsPresented: Binding<Bool>

    var body: some View {
        Form {
            CloudBackupDetailFormContent(
                manager: manager,
                isCancelled: isCancelled,
                isPasskeyMissing: isPasskeyMissing,
                isUnsupportedPasskeyProvider: isUnsupportedPasskeyProvider,
                shouldShowLoadingState: shouldShowLoadingState,
                progressPresentation: progressPresentation,
                presentationCoordinator: presentationCoordinator,
                recreateConfirmationIsPresented: recreateConfirmationIsPresented,
                reinitializeConfirmationIsPresented: reinitializeConfirmationIsPresented
            )
        }
    }
}

private struct CloudBackupDetailFormContent: View {
    let manager: CloudBackupManager
    let isCancelled: Bool
    let isPasskeyMissing: Bool
    let isUnsupportedPasskeyProvider: Bool
    let shouldShowLoadingState: Bool
    let progressPresentation: CloudBackupDetailProgressPresentation
    let presentationCoordinator: PresentationTransitionCoordinator<CloudBackupDetailPresentation>
    let recreateConfirmationIsPresented: Binding<Bool>
    let reinitializeConfirmationIsPresented: Binding<Bool>

    var body: some View {
        if isUnsupportedPasskeyProvider {
            UnsupportedPasskeyProviderContent(manager: manager)
        } else if isPasskeyMissing {
            MissingPasskeyContent(manager: manager)

            if manager.isDetailInventoryComplete {
                DisableCloudBackupSection(
                    manager: manager,
                    detail: manager.detail,
                    presentationCoordinator: presentationCoordinator
                )
            }
        } else {
            CloudBackupPendingUploadConfirmationSection(manager: manager)

            CloudBackupStatusSection(
                manager: manager,
                isCancelled: isCancelled,
                shouldShowLoadingState: shouldShowLoadingState,
                progressPresentation: progressPresentation,
                presentationCoordinator: presentationCoordinator
            )
            VerificationSection(
                manager: manager,
                presentationCoordinator: presentationCoordinator,
                recreateConfirmationIsPresented: recreateConfirmationIsPresented,
                reinitializeConfirmationIsPresented: reinitializeConfirmationIsPresented
            )
            if manager.detail != nil, manager.isDetailInventoryComplete {
                DisableCloudBackupSection(
                    manager: manager,
                    detail: manager.detail,
                    presentationCoordinator: presentationCoordinator
                )
            }
        }
    }
}

private struct CloudBackupStatusSection: View {
    let manager: CloudBackupManager
    let isCancelled: Bool
    let shouldShowLoadingState: Bool
    let progressPresentation: CloudBackupDetailProgressPresentation
    let presentationCoordinator: PresentationTransitionCoordinator<CloudBackupDetailPresentation>

    @AccessibilityFocusState private var inventoryErrorFocused: Bool

    var body: some View {
        Group {
            if progressPresentation == .inventoryInline {
                CloudBackupInventoryCheckingSection()
            }

            if progressPresentation == .verificationInline {
                CloudBackupVerificationInlineSection()
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
                isCancelled: isCancelled,
                shouldShowLoadingState: shouldShowLoadingState,
                progressPresentation: progressPresentation,
                presentationCoordinator: presentationCoordinator
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

                Text("Refreshing the wallet backup list...")
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
    let isCancelled: Bool
    let shouldShowLoadingState: Bool
    let progressPresentation: CloudBackupDetailProgressPresentation
    let presentationCoordinator: PresentationTransitionCoordinator<CloudBackupDetailPresentation>

    var body: some View {
        if progressPresentation == .verificationCard {
            CloudBackupVerificationCard()
        } else if let detail = manager.detail, !isCancelled {
            DetailFormContent(
                detail: detail,
                syncHealth: manager.syncHealth,
                manager: manager,
                presentationCoordinator: presentationCoordinator
            )
        } else if shouldShowLoadingState, manager.detailError == nil {
            CloudBackupLoadingSection()
        }
    }
}

private struct CloudBackupVerificationCard: View {
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

private struct CloudBackupVerificationInlineSection: View {
    var body: some View {
        Section {
            HStack {
                ProgressView()
                    .padding(.trailing, 8)

                Text("Verifying backup integrity...")
            }
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

private struct UnsupportedPasskeyProviderContent: View {
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
