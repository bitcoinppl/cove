import CoveCore
import Foundation
import Observation

/// Writes the cloud backup manager's state whenever it changes, so a UI test can check what the
/// app holds rather than what the lazily built Cloud Backup form happens to have on screen
///
/// A scenario test that names `COVE_CLOUD_BACKUP_UI_DELETE_LOG` gets it in
/// `<COVE_CLOUD_BACKUP_UI_DELETE_LOG>.state`, and a test with a control directory gets it in that
/// directory's `app-state.json`
@MainActor
enum CloudBackupUITestStateProbe {
    static func start() async {
        guard let path = outputPath() else { return }

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

        observe(path: path)

        // rewrites the latest state on a slow timer too, so a failed write doesn't leave the file
        // stale until the next manager change
        while !Task.isCancelled {
            try? await Task.sleep(for: .milliseconds(500))
            write(snapshot(CloudBackupManager.shared), to: path)
        }
    }

    private static func outputPath() -> String? {
        let environment = ProcessInfo.processInfo.environment
        if let logPath = environment["COVE_CLOUD_BACKUP_UI_DELETE_LOG"] {
            return logPath + ".state"
        }

        guard let directory = environment[CloudBackupUITestControl.environmentKey], !directory.isEmpty else {
            return nil
        }

        return (directory as NSString).appendingPathComponent(CloudBackupUITestControl.appStateFileName)
    }

    private static func observe(path: String) {
        let state = withObservationTracking {
            snapshot(CloudBackupManager.shared)
        } onChange: {
            // onChange fires before the new value is set, so read it once the change has landed
            Task { @MainActor in observe(path: path) }
        }

        write(state, to: path)
    }

    private static func write(_ state: CloudBackupUITestAppState, to path: String) {
        // atomic, since the test polls the file while the host writes it
        do {
            try JSONEncoder().encode(state).write(to: URL(fileURLWithPath: path), options: .atomic)
        } catch {
            print("failed to write the cloud backup UI test state: \(error)")
        }
    }

    private static func snapshot(_ manager: CloudBackupManager) -> CloudBackupUITestAppState {
        let detail = switch manager.configuredState?.detail {
        case nil: "none"
        case .notLoaded: "notLoaded"
        case .checking: "checking"
        case .complete: manager.isDetailInventoryComplete ? "complete" : "completeUnconfirmed"
        case .failed: "failed"
        }

        let operation = switch manager.cloudOnlyOperation {
        case .idle: "idle"
        case .operating: "operating"
        case .warning: "warning"
        case .failed: "failed"
        }

        var cloudOnly: [String] = []
        var cloudOnlyIncomplete = false
        if case let .loaded(wallets, incomplete) = manager.cloudOnly {
            cloudOnly = wallets.map(\.recordId)
            cloudOnlyIncomplete = incomplete
        }

        return CloudBackupUITestAppState(
            detail: detail,
            operation: operation,
            cloudOnly: cloudOnly,
            cloudOnlyIncomplete: cloudOnlyIncomplete,
            inventoryReady: manager.isDetailInventoryReady,
            inventoryComplete: manager.isDetailInventoryComplete,
            inventoryError: manager.detailError,
            restoreAll: restoreAllDescription(manager.restoreAllState)
        )
    }

    private static func restoreAllDescription(_ state: CloudBackupRestoreAllState) -> String {
        switch state {
        case .notShown: "notShown"
        case let .startDisabled(count): "startDisabled(\(count))"
        case let .startAvailable(count): "startAvailable(\(count))"
        case let .retryDisabled(count): "retryDisabled(\(count))"
        case let .retryAvailable(count): "retryAvailable(\(count))"
        case let .running(completed, total, _, _): "running(\(completed)/\(total))"
        }
    }
}
