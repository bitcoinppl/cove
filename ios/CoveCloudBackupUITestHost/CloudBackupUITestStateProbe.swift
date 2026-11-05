import CoveCore
import Foundation
import Observation

/// Writes the cloud backup manager's detail inventory state to the file
/// `<COVE_CLOUD_BACKUP_UI_DELETE_LOG>.state` whenever it changes, so a UI test can check what the
/// app holds rather than what the lazily built Cloud Backup form happens to have on screen
@MainActor
enum CloudBackupUITestStateProbe {
    static func start() {
        guard let logPath = ProcessInfo.processInfo.environment["COVE_CLOUD_BACKUP_UI_DELETE_LOG"] else {
            return
        }

        observe(path: logPath + ".state")
    }

    private static func observe(path: String) {
        let snapshot = withObservationTracking {
            Snapshot(CloudBackupManager.shared)
        } onChange: {
            // onChange fires before the new value is set, so read it once the change has landed
            Task { @MainActor in observe(path: path) }
        }

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
