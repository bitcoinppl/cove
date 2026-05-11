import Foundation
import SwiftUI

extension WeakReconciler: CloudBackupManagerReconciler where Reconciler == CloudBackupManager {}

@Observable
final class CloudBackupManager: AnyReconciler, CloudBackupManagerReconciler, @unchecked Sendable {
    static let shared = CloudBackupManager()
    private static let staleVerificationThreshold: TimeInterval = 60 * 60 * 24 * 30

    typealias Action = CloudBackupManagerAction
    typealias Message = CloudBackupReconcileMessage

    @ObservationIgnored let rust: RustCloudBackupManager
    @ObservationIgnored private let rustBridge: DispatchQueue
    @ObservationIgnored private let syncHealthObserver: SyncHealthObserver

    var state: CloudBackupState

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

    var restoreProgress: CloudBackupRestoreProgress? {
        guard case let .restoring(flow) = state.lifecycle else { return nil }
        return flow.progress
    }

    var restoreReport: CloudBackupRestoreReport? {
        switch state.lifecycle {
        case let .restoring(flow):
            flow.report
        case let .configured(configured):
            configured.lastRestoreReport
        case let .failed(failure):
            failure.restoreReport
        default:
            nil
        }
    }

    var syncError: String? {
        switch syncState {
        case let .blocked(message), let .failed(message):
            message
        default:
            nil
        }
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
        if case .required = verificationState { return true }
        return false
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

    var lastVerifiedAt: Date? {
        guard case let .verified(report: _, lastVerifiedAt: lastVerifiedAt) = verificationState else { return nil }
        guard let lastVerifiedAt else { return nil }
        return Date(timeIntervalSince1970: TimeInterval(lastVerifiedAt))
    }

    var isVerificationStale: Bool {
        guard isCloudBackupAvailable, !isUnverified else { return false }
        guard let lastVerifiedAt else { return true }
        return Date.now.timeIntervalSince(lastVerifiedAt) >= Self.staleVerificationThreshold
    }

    var detail: CloudBackupDetail? {
        guard case let .loaded(state: loaded) = configuredState?.detail else { return nil }
        return loaded.detail
    }

    var verificationPresentation: CloudBackupVerificationPresentation {
        configuredState?.verificationPresentation ?? .hidden(source: nil)
    }

    var cloudOnly: CloudOnlyState {
        switch configuredState?.detail {
        case nil, .notLoaded:
            .notFetched
        case .loading:
            .loading
        case let .loaded(state: loaded):
            loaded.cloudOnly
        case let .failed(error):
            .failed(error: error)
        }
    }

    var cloudOnlyOperation: CloudOnlyOperation {
        guard case let .loaded(state: loaded) = configuredState?.detail else { return .idle }
        return loaded.cloudOnlyOperation
    }

    var otherBackupsOperation: OtherBackupsOperation {
        guard case let .loaded(state: loaded) = configuredState?.detail else { return .idle }
        return loaded.otherBackupsOperation
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

    private func apply(_ message: Message) {
        switch message {
        case let .lifecycle(lifecycle):
            state.lifecycle = lifecycle
        }
    }

    func reconcile(message: Message) {
        DispatchQueue.main.async { [weak self] in
            self?.apply(message)
        }
    }

    func reconcileMany(messages: [Message]) {
        DispatchQueue.main.async { [weak self] in
            messages.forEach { self?.apply($0) }
        }
    }
}
