import Foundation
import SwiftUI

extension WeakReconciler: CloudBackupManagerReconciler where Reconciler == CloudBackupManager {}

/// Holds the Cloud Backup screen Rust projects and what it can restore, and forwards user intents
/// to Rust
///
/// Views read only `screen` and `recovery`: which body and sections are on screen, whether each
/// control is enabled, and whether a wallet is backed up, are Rust's decisions. The manager derives
/// nothing from the screen's content
@Observable
final class CloudBackupManager: ReconcilingManager, CloudBackupManagerReconciler, @unchecked Sendable {
    static let shared = CloudBackupManager()

    typealias Action = CloudBackupManagerAction
    typealias Message = CloudBackupReconcileMessage

    @ObservationIgnored let rust: RustCloudBackupManager
    @ObservationIgnored private let rustBridge: DispatchQueue
    @ObservationIgnored private let syncHealthObserver: SyncHealthObserver

    /// The screen Rust last published, with its revision; every update replaces it in one assignment
    private(set) var published: CloudBackupPublishedScreen

    /// What Cloud Backup can restore, with its revision, read from the persisted record so it is
    /// already right at launch, before the first screen publish
    private(set) var publishedRecovery: CloudBackupPublishedRecovery
    var enableCompletion: TaggedItem<CloudBackupEnableContext>?

    var screen: CloudBackupScreen {
        published.screen
    }

    /// Whether Cloud Backup can restore each wallet, and whether a backup is configured at all;
    /// views ask this, never whether the screen shows a configured backup
    var recovery: CloudBackupRecovery {
        publishedRecovery.recovery
    }

    /// A manager that shows `published` and is not listening yet; the shared one listens next
    init(
        rust: RustCloudBackupManager,
        published: CloudBackupPublishedScreen,
        recovery: CloudBackupPublishedRecovery = .init(revision: 0, recovery: .init(configured: false, confirmedWallets: []))
    ) {
        let rustBridge = DispatchQueue(label: "cove.CloudBackupManager.rustbridge", qos: .userInitiated)
        self.rust = rust
        self.rustBridge = rustBridge
        self.published = published
        self.publishedRecovery = recovery
        self.syncHealthObserver = ICloudDriveHelper.shared.makeSyncHealthObserver {
            rustBridge.async { rust.cloudStorageDidChange() }
        }
    }

    private convenience init() {
        let rust = RustCloudBackupManager()

        // the snapshots are read before listening, so the updates still queued from before them
        // are at or before their revisions, and applying skips them instead of replaying them
        self.init(rust: rust, published: rust.state(), recovery: rust.recovery())
        rust.listenForUpdates(reconciler: WeakReconciler(self))
        syncHealthObserver.start()
        // Keep the initial iCloud health probe off the main startup path.
        rustBridge.async { rust.cloudStorageDidChange() }
    }

    func dispatch(action: Action) {
        dispatch(action)
    }

    func dispatch(_ action: Action) {
        rustBridge.async { self.rust.dispatch(action: action) }
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
        case let .screen(delta):
            // one assignment, so no frame shows the new body with the old actions
            published = published.applying(delta: delta)

            // a completion from an enable that has since been turned off, such as by a full wipe, is stale
            if case .disabled = screen.body { enableCompletion = nil }

        case let .enableCompleted(context):
            enableCompletion = TaggedItem(context)

        case let .recovery(update):
            publishedRecovery = publishedRecovery.applying(update: update)

        // iCloud Drive has no account binding to release, so settling off needs nothing from iOS
        case .settledOff:
            break

        // drive account switching is Android-only and iOS never dispatches a switch, so Rust
        // never asks iOS for these platform steps
        case .driveAccountSwitchSelectionRequired,
             .driveAccountSwitchStateRequired,
             .driveAccountSwitchCommitRequired,
             .driveAccountSwitchRollbackRequired,
             .driveAccountSwitchFinalizeRequired:
            break
        }
    }
}
