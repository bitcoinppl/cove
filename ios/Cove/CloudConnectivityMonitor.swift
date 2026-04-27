import Network
import os

final class CloudConnectivityMonitor: ConnectivityAccess, @unchecked Sendable {
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

        let initialConnected = monitor.currentPath.status == .satisfied
        setConnected(initialConnected)
        updateRustConnectivity(initialConnected)
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
        RustConnectivityManager().setConnectionState(isConnected: isConnected)
    }
}
