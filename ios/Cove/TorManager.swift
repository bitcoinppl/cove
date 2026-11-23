import SwiftUI

/// Holds Tor's saved mode and status for Settings, and forwards the user's Tor choices to Rust
///
/// `RustTorManager` is a process singleton, so this is one too, shared the way
/// `CloudBackupManager.shared` is. Rust owns validation, saving and what Tor is doing; this only
/// mirrors the values Rust reconciles
@Observable @MainActor
final class TorManager: TorManagerReconciler {
    static let shared = TorManager()

    @ObservationIgnored private let rust: RustTorManager

    /// The saved mode, nil when Rust can't read the saved setting
    private(set) var mode: TorMode?
    private(set) var status: TorStatus

    /// The last mode change Rust refused, which the Tor screen shows until the user dismisses it
    private(set) var modeRejection: TaggedItem<TorManagerError>?

    private init() {
        let rust = RustTorManager()
        let state = rust.state()
        self.rust = rust
        mode = state.mode
        status = state.status

        rust.listenForUpdates(reconciler: self)

        // Rust forwards only the status changes after the first listener, so one between the
        // first snapshot and listening would be lost without this second read
        let current = rust.state()
        mode = current.mode
        status = current.status
    }

    /// Rust calls this from its own threads; the main queue applies messages in the order Rust
    /// sent them, so a stale status never lands after a newer one
    nonisolated func reconcile(message: TorManagerReconcileMessage) {
        DispatchQueue.main.async {
            MainActor.assumeIsolated { self.apply(message) }
        }
    }

    private func apply(_ message: TorManagerReconcileMessage) {
        switch message {
        case let .mode(mode):
            self.mode = mode
        case let .status(status):
            self.status = status
        case let .modeRejected(error):
            modeRejection = TaggedItem(error)
        }
    }

    /// Saving happens off the caller's thread; the result arrives as a mode or a rejection
    func dispatch(action: TorManagerAction) {
        rust.dispatch(action: action)
    }

    func dismissModeRejection() {
        modeRejection = nil
    }

    /// The proxy address the form starts with when no proxy is saved
    func defaultProxyAddress() -> String {
        rust.defaultProxyAddress()
    }

    /// The address in the form the app saves, or why it can't be used, in words for the user
    func validateProxyAddress(_ input: String) -> Result<String, TorManagerError> {
        do {
            return try .success(rust.validateProxyAddress(input: input))
        } catch let error as TorManagerError {
            return .failure(error)
        } catch {
            return .failure(.InvalidProxy(String(describing: error)))
        }
    }

    /// Asks the Tor Project's check service, over Tor, whether traffic leaves through Tor
    func checkConnection() async -> Result<TorConnectionCheck, TorManagerError> {
        do {
            return try await .success(rust.checkConnection())
        } catch let error as TorManagerError {
            return .failure(error)
        } catch {
            return .failure(.CheckFailed(String(describing: error)))
        }
    }
}

extension TorMode {
    /// The saved proxy's address, nil for the modes without one
    var proxyAddress: String? {
        guard case let .proxy(address) = self else { return nil }
        return address
    }
}
