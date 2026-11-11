import Foundation
import SwiftUI

extension WeakReconciler: CloudBackupManagerReconciler where Reconciler == CloudBackupManager {}

/// Holds the Cloud Backup screen Rust projects and forwards user intents to Rust
///
/// Views read only `screen`: which body and sections are on screen, and whether each control is
/// enabled, are Rust's decisions. The manager derives nothing from the screen's content
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
    var enableCompletion: TaggedItem<CloudBackupEnableContext>?

    var screen: CloudBackupScreen {
        published.screen
    }

    /// A manager that shows `published` and is not listening yet; the shared one listens next
    init(rust: RustCloudBackupManager, published: CloudBackupPublishedScreen) {
        let rustBridge = DispatchQueue(label: "cove.CloudBackupManager.rustbridge", qos: .userInitiated)
        self.rust = rust
        self.rustBridge = rustBridge
        self.published = published
        self.syncHealthObserver = ICloudDriveHelper.shared.makeSyncHealthObserver {
            rustBridge.async { rust.cloudStorageDidChange() }
        }
    }

    private convenience init() {
        let rust = RustCloudBackupManager()

        // the snapshot is read before listening, so the deltas still queued from before it are
        // at or before its revision, and applying skips them instead of replaying them over it
        self.init(rust: rust, published: rust.state())
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

        // drive account switching is Android-only, so iOS intentionally ignores these messages
        case .driveAccountSwitchCommitRequired,
             .driveAccountSwitchRollbackRequired,
             .driveAccountSwitchFinalizeRequired:
            break
        }
    }
}
