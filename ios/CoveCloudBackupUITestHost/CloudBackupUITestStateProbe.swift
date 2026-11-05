import CoveCore
import Foundation
import Observation

/// Writes the cloud backup manager's detail inventory state to the file
/// `<COVE_CLOUD_BACKUP_UI_DELETE_LOG>.state` whenever it changes, so a UI test can check what the
/// app holds rather than what the lazily built Cloud Backup form happens to have on screen
@MainActor
enum CloudBackupUITestStateProbe {
    static func start() async {
        guard let logPath = ProcessInfo.processInfo.environment["COVE_CLOUD_BACKUP_UI_DELETE_LOG"] else {
            return
        }

        // creating CloudBackupManager.shared spawns rust actors, so wait until bootstrap has set
        // its last step (rust/src/bootstrap.rs:132); the app's runBootstrap then resumes on the
        // main actor and its completeBootstrap creates the shared manager (ios/Cove/CoveApp.swift:171
        // and :346), which the pause leaves it time to do first
        do {
            while bootstrapProgress() != .complete {
                try await Task.sleep(for: .milliseconds(100))
            }
            try await Task.sleep(for: .seconds(1))
        } catch {
            return
        }

        let path = logPath + ".state"
        observe(path: path)

        // rewrites the latest state on a slow timer too, so a failed write doesn't leave the file
        // stale until the next manager change
        while !Task.isCancelled {
            try? await Task.sleep(for: .milliseconds(500))
            write(Snapshot(CloudBackupManager.shared), to: path)
        }
    }

    private static func observe(path: String) {
        let snapshot = withObservationTracking {
            Snapshot(CloudBackupManager.shared)
        } onChange: {
            // onChange fires before the new value is set, so read it once the change has landed
            Task { @MainActor in observe(path: path) }
        }

        write(snapshot, to: path)
    }

    private static func write(_ snapshot: Snapshot, to path: String) {
        // atomic, since the test polls the file while the host writes it
        do {
            try JSONEncoder().encode(snapshot).write(to: URL(fileURLWithPath: path), options: .atomic)
        } catch {
            print("failed to write the cloud backup UI test state: \(error)")
        }
    }

    private struct Snapshot: Encodable {
        /// `none`, `notLoaded`, `checking`, `complete` (provider confirmed), `completeUnconfirmed` or `failed`
        let detail: String
        /// `idle`, `operating`, `warning` or `failed`
        let operation: String
        /// Cloud-only wallet record ids, empty unless the cloud-only list is loaded
        let cloudOnly: [String]
        let cloudOnlyIncomplete: Bool

        init(_ manager: CloudBackupManager) {
            detail = switch manager.configuredState?.detail {
            case nil: "none"
            case .notLoaded: "notLoaded"
            case .checking: "checking"
            case .complete: manager.isDetailInventoryComplete ? "complete" : "completeUnconfirmed"
            case .failed: "failed"
            }

            operation = switch manager.cloudOnlyOperation {
            case .idle: "idle"
            case .operating: "operating"
            case .warning: "warning"
            case .failed: "failed"
            }

            if case let .loaded(wallets, incomplete) = manager.cloudOnly {
                cloudOnly = wallets.map(\.recordId)
                cloudOnlyIncomplete = incomplete
            } else {
                cloudOnly = []
                cloudOnlyIncomplete = false
            }
        }
    }
}
