import SwiftUI

enum CloudBackupDetailProgressPresentation: Equatable {
    case none
    case inventoryInline
    case verificationCard
    case verificationInline
}

func cloudBackupDetailProgressPresentation(
    verificationState: CloudBackupVerificationState?,
    isInventoryChecking: Bool,
    hasRetainedDetail: Bool,
    hasVisibleWalletRows: Bool
) -> CloudBackupDetailProgressPresentation {
    if case .running = verificationState {
        return hasVisibleWalletRows ? .verificationInline : .verificationCard
    }

    if isInventoryChecking, hasRetainedDetail {
        return .inventoryInline
    }

    return .none
}

func cloudBackupHasVisibleWalletRows(
    detail: CloudBackupDetail?,
    cloudOnly: CloudOnlyState
) -> Bool {
    guard let detail else { return false }

    if !detail.upToDate.isEmpty || !detail.needsSync.isEmpty {
        return true
    }

    guard case let .loaded(wallets) = cloudOnly else { return false }
    return !wallets.isEmpty
}

struct CloudBackupDetailScreen: View {
    @Environment(\.dismiss) private var dismiss
    @Environment(CloudBackupPresentationCoordinator.self)
    private var cloudBackupPresentationCoordinator
    @State private var manager = CloudBackupManager.shared
    @State private var showRecreateConfirmation = false
    @State private var showReinitializeConfirmation = false

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
        manager.detail == nil && !hasVerificationResult && !isCancelled
    }

    private var progressPresentation: CloudBackupDetailProgressPresentation {
        cloudBackupDetailProgressPresentation(
            verificationState: manager.verificationState,
            isInventoryChecking: manager.isDetailInventoryChecking,
            hasRetainedDetail: manager.detail != nil,
            hasVisibleWalletRows: cloudBackupHasVisibleWalletRows(
                detail: manager.detail,
                cloudOnly: manager.cloudOnly
            )
        )
    }

    private var hasCloudBackupPresentationBlocker: Bool {
        showRecreateConfirmation || showReinitializeConfirmation
    }

    var body: some View {
        CloudBackupReinitializeDialog(
            manager: manager,
            isPresented: $showReinitializeConfirmation,
            content: CloudBackupRecreateDialog(
                manager: manager,
                isPresented: $showRecreateConfirmation,
                content: CloudBackupDetailForm(
                    manager: manager,
                    isCancelled: isCancelled,
                    isPasskeyMissing: isPasskeyMissing,
                    isUnsupportedPasskeyProvider: isUnsupportedPasskeyProvider,
                    shouldShowLoadingState: shouldShowLoadingState,
                    progressPresentation: progressPresentation,
                    onRecreate: showRecreateDialog,
                    onReinitialize: showReinitializeDialog
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
            )
        )
    }

    private func enterDetail() async {
        manager.dispatch(action: .enterDetail)
    }

    private func closeDetail() {
        manager.dispatch(action: .closeDetail)
        cloudBackupPresentationCoordinator.setBlocker(.cloudBackupDetailDialog, active: false)
    }

    private func showRecreateDialog() {
        guard manager.isDetailInventoryComplete else { return }

        showRecreateConfirmation = true
    }

    private func showReinitializeDialog() {
        guard manager.isDetailInventoryComplete else { return }

        showReinitializeConfirmation = true
    }
}

private struct CloudBackupDetailForm: View {
    let manager: CloudBackupManager
    let isCancelled: Bool
    let isPasskeyMissing: Bool
    let isUnsupportedPasskeyProvider: Bool
    let shouldShowLoadingState: Bool
    let progressPresentation: CloudBackupDetailProgressPresentation
    let onRecreate: () -> Void
    let onReinitialize: () -> Void

    var body: some View {
        Form {
            CloudBackupDetailFormContent(
                manager: manager,
                isCancelled: isCancelled,
                isPasskeyMissing: isPasskeyMissing,
                isUnsupportedPasskeyProvider: isUnsupportedPasskeyProvider,
                shouldShowLoadingState: shouldShowLoadingState,
                progressPresentation: progressPresentation,
                onRecreate: onRecreate,
                onReinitialize: onReinitialize
            )
        }
    }
}

private struct CloudBackupRecreateDialog<Content: View>: View {
    let manager: CloudBackupManager
    @Binding var isPresented: Bool
    let content: Content

    var body: some View {
        content.confirmationDialog(
            "Recreate Backup Index",
            isPresented: $isPresented,
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
    }
}

private struct CloudBackupReinitializeDialog<Content: View>: View {
    let manager: CloudBackupManager
    @Binding var isPresented: Bool
    let content: Content

    var body: some View {
        content.confirmationDialog(
            "Reinitialize Cloud Backup",
            isPresented: $isPresented,
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
    let isCancelled: Bool
    let isPasskeyMissing: Bool
    let isUnsupportedPasskeyProvider: Bool
    let shouldShowLoadingState: Bool
    let progressPresentation: CloudBackupDetailProgressPresentation
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
                isCancelled: isCancelled,
                shouldShowLoadingState: shouldShowLoadingState,
                progressPresentation: progressPresentation
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
    let isCancelled: Bool
    let shouldShowLoadingState: Bool
    let progressPresentation: CloudBackupDetailProgressPresentation

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
                progressPresentation: progressPresentation
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

    var body: some View {
        if progressPresentation == .verificationCard {
            CloudBackupVerificationCard()
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
