import Foundation

@_exported import CoveCore
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
    }

    var status: CloudBackupStatus {
        state.status
    }

    var promptIntent: CloudBackupPromptIntent {
        state.promptIntent
    }

    var connectivityHint: CloudConnectivityHint {
        state.connectivityHint
    }

    var syncHealth: CloudSyncHealth {
        state.syncHealth
    }

    var progress: (completed: UInt32, total: UInt32)? {
        state.progress.map { ($0.completed, $0.total) }
    }

    var restoreProgress: CloudBackupRestoreProgress? {
        state.restoreProgress
    }

    var restoreReport: CloudBackupRestoreReport? {
        state.restoreReport
    }

    var syncError: String? {
        state.syncError
    }

    var hasPendingUploadVerification: Bool {
        state.hasPendingUploadVerification
    }

    var isBackgroundVerifying: Bool {
        guard hasPendingUploadVerification else { return false }
        if case .verifying = verification { return true }
        return false
    }

    var shouldPromptVerification: Bool {
        if isBackgroundVerifying { return false }
        return state.shouldPromptVerification
    }

    var isUnverified: Bool {
        if isBackgroundVerifying { return false }
        if case .needsVerification = state.verificationMetadata { return true }

        return false
    }

    var isConfigured: Bool {
        switch state.verificationMetadata {
        case .notConfigured: false
        case .configuredNeverVerified, .verified, .needsVerification: true
        }
    }

    var isCloudBackupEnabled: Bool {
        rust.isCloudBackupEnabled()
    }

    var lastVerifiedAt: Date? {
        guard case let .verified(lastVerifiedAt) = state.verificationMetadata else { return nil }
        return Date(timeIntervalSince1970: TimeInterval(lastVerifiedAt))
    }

    var isVerificationStale: Bool {
        guard case .enabled = status, !isUnverified else { return false }
        guard let lastVerifiedAt else { return true }
        return Date.now.timeIntervalSince(lastVerifiedAt) >= Self.staleVerificationThreshold
    }

    var detail: CloudBackupDetail? {
        state.detail
    }

    var verification: VerificationState {
        state.verification
    }

    var sync: SyncState {
        state.sync
    }

    var recovery: RecoveryState {
        state.recovery
    }

    var cloudOnly: CloudOnlyState {
        state.cloudOnly
    }

    var cloudOnlyOperation: CloudOnlyOperation {
        state.cloudOnlyOperation
    }

    func dispatch(action: Action) {
        dispatch(action)
    }

    func dispatch(_ action: Action) {
        rustBridge.async { self.rust.dispatch(action: action) }
    }

    private func apply(_ message: Message) {
        switch message {
        case let .status(status):
            state.status = status
        case let .connectivityHint(connectivityHint):
            state.connectivityHint = connectivityHint
        case let .syncHealth(syncHealth):
            state.syncHealth = syncHealth
        case let .promptIntent(promptIntent):
            state.promptIntent = promptIntent
        case let .progress(progress):
            state.progress = progress
        case let .restoreProgress(progress):
            state.restoreProgress = progress
        case let .restoreReport(report):
            state.restoreReport = report
        case let .syncError(syncError):
            state.syncError = syncError
        case let .verificationPrompt(pending):
            state.shouldPromptVerification = pending
        case let .verificationMetadata(verificationMetadata):
            state.verificationMetadata = verificationMetadata
        case let .pendingUploadVerification(pending):
            state.hasPendingUploadVerification = pending
        case let .detail(detail):
            state.detail = detail
        case let .verification(verification):
            state.verification = verification
        case let .sync(sync):
            state.sync = sync
        case let .recovery(recovery):
            state.recovery = recovery
        case let .cloudOnly(cloudOnly):
            state.cloudOnly = cloudOnly
        case let .cloudOnlyOperation(cloudOnlyOperation):
            state.cloudOnlyOperation = cloudOnlyOperation
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
