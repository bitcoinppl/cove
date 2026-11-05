import Network
import os

/// Connectivity the app starts once bootstrap has finished, after which it reports every change
/// to Rust
///
/// Production uses `CloudConnectivityMonitor`; the Cloud Backup UI test host swaps in a scripted
/// network so a test can drop and restore it without touching the simulator's network
protocol LifecycleConnectivityAccess: ConnectivityAccess {
    func start()
}

final class CloudConnectivityMonitor: LifecycleConnectivityAccess, @unchecked Sendable {
    static let shared = CloudConnectivityMonitor()

    private struct State {
        var started = false
        var isConnected = true
    }

    private let monitor = NWPathMonitor()
    private let queue = DispatchQueue(label: "cove.CloudConnectivityMonitor")
    private let state = OSAllocatedUnfairLock(initialState: State())

    private init() {}

    func start() {
        guard markStartedIfNeeded() else { return }

        monitor.pathUpdateHandler = { [weak self] path in
            guard let self else { return }
            let isConnected = path.status == .satisfied
            setConnected(isConnected)
            updateRustConnectivity(isConnected)
        }

        monitor.start(queue: queue)
    }

    func isConnected() -> Bool {
        state.withLock { $0.isConnected }
    }

    private func markStartedIfNeeded() -> Bool {
        state.withLock { state in
            guard !state.started else { return false }
            state.started = true
            return true
        }
    }

    private func setConnected(_ isConnected: Bool) {
        state.withLock { $0.isConnected = isConnected }
    }

    private func updateRustConnectivity(_ isConnected: Bool) {
        let status: ConnectivityStatus = isConnected ? .connected : .disconnected
        RustConnectivityManager().setConnectionStatus(status: status)
    }
}
