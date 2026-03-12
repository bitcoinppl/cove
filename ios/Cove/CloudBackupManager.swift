import Foundation

@_exported import CoveCore
import SwiftUI

@Observable
final class CloudBackupManager: CloudBackupManagerReconciler, @unchecked Sendable {
    static let shared = CloudBackupManager()

    let rust: RustCloudBackupManager
    var state: CloudBackupState = .disabled
    var progress: (completed: UInt32, total: UInt32)?
    var restoreReport: CloudBackupRestoreReport?

    private init() {
        self.rust = RustCloudBackupManager()
        self.rust.listenForUpdates(reconciler: self)
    }

    func reconcile(message: CloudBackupReconcileMessage) {
        DispatchQueue.main.async { [weak self] in
            guard let self else { return }

            switch message {
            case let .stateChanged(newState):
                self.state = newState
            case let .progressUpdated(completed, total):
                self.progress = (completed, total)
            case .enableComplete:
                self.progress = nil
            case let .restoreComplete(report):
                self.restoreReport = report
                self.progress = nil
            }
        }
    }
}
