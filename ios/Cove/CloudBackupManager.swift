import Foundation
import SwiftUI

extension WeakReconciler: CloudBackupManagerReconciler where Reconciler == CloudBackupManager {}

extension CloudBackupDetailState {
    var retainedDetailState: LoadedCloudBackupDetail? {
        switch self {
        case let .complete(state): state
        case let .checking(retained): retained
        case let .failed(_, _, retained): retained
        case .notLoaded: nil
        }
    }

    var inventoryError: String? {
        guard case let .failed(_, error, _) = self else { return nil }
        return error
    }

    var isChecking: Bool {
        guard case .checking = self else { return false }
        return true
    }

    var isComplete: Bool {
        guard case .complete = self else { return false }
        return true
    }
}

@Observable
final class CloudBackupManager: ReconcilingManager, CloudBackupManagerReconciler, @unchecked Sendable {
    static let shared = CloudBackupManager()

    typealias Action = CloudBackupManagerAction
    typealias Message = CloudBackupReconcileMessage

    @ObservationIgnored let rust: RustCloudBackupManager
    @ObservationIgnored private let rustBridge: DispatchQueue
    @ObservationIgnored private let syncHealthObserver: SyncHealthObserver

    var state: CloudBackupState
    var enableCompletion: TaggedItem<CloudBackupEnableContext>?

    private init() {
        let rust = RustCloudBackupManager()
        let rustBridge = DispatchQueue(label: "cove.CloudBackupManager.rustbridge", qos: .userInitiated)
        self.rust = rust
        self.rustBridge = rustBridge
        self.state = rust.state()
        self.syncHealthObserver = ICloudDriveHelper.shared.makeSyncHealthObserver {
            rustBridge.async { rust.cloudStorageDidChange() }
        }
        rust.listenForUpdates(reconciler: WeakReconciler(self))
        syncHealthObserver.start()
        // Keep the initial iCloud health probe off the main startup path.
        rustBridge.async { rust.cloudStorageDidChange() }
    }

    var lifecycle: CloudBackupLifecycle {
        state.lifecycle
    }

    var settingsRowStatus: CloudBackupSettingsRowStatus {
        state.settingsRowStatus
    }

    var configuredState: CloudBackupConfiguredState? {
        guard case let .configured(configured) = state.lifecycle else { return nil }
        return configured
    }

    var enableFlow: CloudBackupEnableFlow? {
        guard case let .enabling(flow) = state.lifecycle else { return nil }
        return flow
    }

    var passkeyState: CloudBackupPasskeyState? {
        configuredState?.passkey
    }

    var passkeyRepairState: CloudBackupPasskeyRepairState? {
        guard case let .needsRepair(state) = passkeyState else { return nil }
        return state
    }

    var verificationState: CloudBackupVerificationState? {
        configuredState?.verification
    }

    var syncState: CloudBackupSyncState? {
        configuredState?.sync
    }

    var lifecycleFailureMessage: String? {
        guard case let .failed(failure) = state.lifecycle else { return nil }
        return failure.message
    }

    var pendingEnableRecovery: CloudBackupPendingEnableRecovery? {
        guard case let .pendingEnableRecovery(recovery) = state.lifecycle else { return nil }
        return recovery
    }

    var isLifecycleDisabled: Bool {
        if case .disabled = state.lifecycle { return true }
        return false
    }

    var isLifecycleEnabling: Bool {
        if case .enabling = state.lifecycle { return true }
        return false
    }

    var isLifecycleRestoring: Bool {
        if case .restoring = state.lifecycle { return true }
        return false
    }

    var isLifecycleConfigured: Bool {
        configuredState != nil
    }

    var isCloudBackupAvailable: Bool {
        guard case .available = passkeyState else { return false }
        return true
    }

    var isPasskeyMissing: Bool {
        switch passkeyState {
        case .missing, .needsRepair:
            true
        default:
            false
        }
    }

    var isUnsupportedPasskeyProvider: Bool {
        guard case .unsupportedProvider = passkeyState else { return false }
        return true
    }

    var rootPrompt: CloudBackupRootPrompt {
        switch state.lifecycle {
        case let .enabling(.awaitingForceNewConfirmation(context, passkeyHint)):
            .existingBackupFound(context, passkeyHint)
        case let .enabling(.awaitingPasskeyChoice(intent)):
            .passkeyChoice(intent)
        case let .configured(configured):
            configured.rootPrompt
        default:
            .none
        }
    }

    var syncHealth: CloudSyncHealth {
        configuredState?.syncHealth ?? .unknown
    }

    var progress: (completed: UInt32, total: UInt32)? {
        let progress: CloudBackupProgress? = switch enableFlow {
        case let .uploadingInitialBackup(progress), let .retryingUploadWithStagedMaterial(progress):
            progress
        default:
            nil
        }

        return progress.map { ($0.completed, $0.total) }
    }

    var syncError: String? {
        switch syncState {
        case let .blocked(message), let .failed(message):
            message
        default:
            nil
        }
    }

    var destructiveOperationState: CloudBackupDestructiveOperationState {
        configuredState?.destructiveOperation ?? .idle
    }

    var isPerformingDestructiveAction: Bool {
        destructiveOperationState != .idle
    }

    var isDisablingCloudBackup: Bool {
        if case .disabling = destructiveOperationState { return true }
        return false
    }

    var disableFailure: (message: String, canKeepEnabled: Bool)? {
        guard case let .disableFailed(message, canKeepEnabled) = destructiveOperationState else {
            return nil
        }

        return (message, canKeepEnabled)
    }

    var hasPendingUploadVerification: Bool {
        if case .awaitingUploadConfirmation = verificationState { return true }
        return false
    }

    var isBackgroundVerifying: Bool {
        hasPendingUploadVerification
    }

    var shouldPromptVerification: Bool {
        if isBackgroundVerifying { return false }
        switch verificationState {
        case .required, .cancelled: return true
        default: return false
        }
    }

    var isUnverified: Bool {
        if isBackgroundVerifying { return false }
        return shouldPromptVerification
    }

    var isConfigured: Bool {
        isLifecycleConfigured
    }

    var isCloudBackupEnabled: Bool {
        rust.isCloudBackupEnabled()
    }

    var detail: CloudBackupDetail? {
        configuredState?.detail.retainedDetailState?.detail
    }

    var detailError: String? {
        configuredState?.detail.inventoryError
    }

    var isDetailInventoryChecking: Bool {
        configuredState?.detail.isChecking == true
    }

    var isDetailInventoryComplete: Bool {
        configuredState?.detail.isComplete == true
    }

    var verificationPresentation: CloudBackupVerificationPresentation {
        configuredState?.verificationPresentation ?? .hidden(source: nil)
    }

    var cloudOnly: CloudOnlyState {
        switch configuredState?.detail {
        case nil, .notLoaded:
            .notFetched
        case let .checking(retained):
            retained?.cloudOnly ?? .loading
        case let .complete(state: loaded):
            loaded.cloudOnly
        case let .failed(_, error, retained):
            retained?.cloudOnly ?? .failed(error: error)
        }
    }

    var cloudOnlyOperation: CloudOnlyOperation {
        configuredState?.detail.retainedDetailState?.cloudOnlyOperation ?? .idle
    }

    var restoreAllState: CloudBackupRestoreAllState {
        configuredState?.restoreAll ?? .notShown
    }

    var otherBackupsOperation: OtherBackupsOperation {
        configuredState?.detail.retainedDetailState?.otherBackupsOperation ?? .idle
    }

    func dispatch(action: Action) {
        dispatch(action)
    }

    func dispatch(_ action: Action) {
        rustBridge.async { self.rust.dispatch(action: action) }
    }

    func startVerification(source: CloudBackupVerificationSource = .settings) {
        dispatch(.startVerification(source))
    }

    func startRestoreAll() {
        dispatch(.startRestoreAll)
    }

    func retryRestoreAllRemaining() {
        dispatch(.retryRestoreAllRemaining)
    }

    func cancelRestoreAll() {
        dispatch(.cancelRestoreAll)
    }

    func consumeEnableCompletion(_ completion: TaggedItem<CloudBackupEnableContext>) {
        guard enableCompletion?.id == completion.id else { return }

        enableCompletion = nil
    }

    func onboardingEnableCompletionReadiness() async -> CloudBackupOnboardingCompletionReadiness {
        await withCheckedContinuation { continuation in
            rustBridge.async {
                continuation.resume(returning: self.rust.onboardingEnableCompletionReadiness())
            }
        }
    }

    func apply(_ message: Message) {
        switch message {
        case let .lifecycle(lifecycle, settingsRowStatus):
            state.lifecycle = lifecycle
            state.settingsRowStatus = settingsRowStatus
        case let .enableCompleted(context):
            enableCompletion = TaggedItem(context)
        case .driveAccountSwitchCommitRequired, .driveAccountSwitchRollbackRequired:
            break
        }
    }
}
