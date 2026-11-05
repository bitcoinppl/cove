import CoveCore
import Foundation
import os

/// Network connectivity the UI test switches on and off through its script
///
/// It reports to Rust the way `CloudConnectivityMonitor` does, and `ControlledCloudStorageAccess`
/// fails every provider call while it is off, so a test sees what a dropped network does to Cloud
/// Backup without touching the simulator's network, which the fake node needs. The provider reads
/// the script on every call while this reads it on a short timer, so the provider fails as soon as
/// the network goes and works again before Rust hears about the reconnect, as a real provider does
final class ControlledConnectivityAccess: LifecycleConnectivityAccess, @unchecked Sendable {
    private static let pollInterval = Duration.milliseconds(100)

    private let controller: CloudBackupUITestController
    private let started = OSAllocatedUnfairLock(initialState: false)

    init(controller: CloudBackupUITestController) {
        self.controller = controller
    }

    func start() {
        let isFirstStart = started.withLock { started in
            defer { started = true }
            return !started
        }
        guard isFirstStart else { return }

        Task.detached(priority: .utility) { [controller] in
            var reported: Bool?
            while !Task.isCancelled {
                let isConnected = controller.script().networkConnected
                if isConnected != reported {
                    let status: ConnectivityStatus = isConnected ? .connected : .disconnected
                    RustConnectivityManager().setConnectionStatus(status: status)
                    controller.recordReportedConnectivity(isConnected)
                    reported = isConnected
                }

                try? await Task.sleep(for: Self.pollInterval)
            }
        }
    }

    func isConnected() -> Bool {
        controller.script().networkConnected
    }
}
