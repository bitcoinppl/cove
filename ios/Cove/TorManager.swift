import SwiftUI

extension WeakReconciler: TorManagerReconciler where Reconciler == TorManager {}

enum TorStatus: Equatable {
    case off
    case bootstrapping(percent: UInt8, message: String)
    case ready
    case stopped
    case failed(message: String)
}

@Observable final class TorManager: ReconcilingManager {
    typealias Message = TorManagerReconcileMessage

    static let shared = makeShared()

    @ObservationIgnored
    private let rust: RustTorManager

    var config: TorConfig = .off
    var status: TorStatus = .off
    var connectionTestStates: [TorTestStep: TorTestState] = [:]
    var builtInFailure: TaggedString?

    /// Built-in Tor was not auto-started because repeated launches failed
    var autoStartSuppressed: Bool = false

    /// Last user-initiated connection test failure, which says nothing about the live route
    var connectionTestError: String?

    var isEnabled: Bool {
        config != .off
    }

    var isConnectionTestRunning: Bool {
        connectionTestStates.values.contains(.running)
    }

    private static func makeShared() -> TorManager {
        requireBootstrapComplete()
        return TorManager()
    }

    private static func requireBootstrapComplete() {
        if ProcessInfo.processInfo.environment["XCODE_RUNNING_FOR_PREVIEWS"] == "1" {
            return
        }

        let step = bootstrapProgress()
        guard step == .complete else {
            fatalError("TorManager initialized before bootstrap completed: \(step)")
        }
    }

    private init() {
        let rust = RustTorManager()
        self.rust = rust

        rust.listenForUpdates(reconciler: WeakReconciler(self))
    }

    @MainActor
    func enable() async throws {
        _ = try await rust.dispatch(action: .enable)

        // enabling an already built-in config is a no-op transition, so no reconcile message
        // clears the suppression notice until the first bootstrap progress arrives
        guard autoStartSuppressed else { return }

        autoStartSuppressed = false
        status = .bootstrapping(percent: 0, message: "Starting Tor")
    }

    @MainActor
    func disable() async throws -> TorDisableWarning? {
        let result = try await rust.dispatch(action: .disable)

        return result.disableWarning
    }

    @MainActor
    func disableConfirmed() async throws {
        _ = try await rust.dispatch(action: .disableConfirmed)
    }

    @MainActor
    func setConfig(_ config: TorConfig) async throws -> TorDisableWarning? {
        let result = try await rust.dispatch(action: .setConfig(config))

        return result.disableWarning
    }

    @MainActor
    func runConnectionTest() async throws {
        connectionTestStates = [:]
        connectionTestError = nil
        _ = try await rust.dispatch(action: .runConnectionTest)
    }

    /// Resynchronizes published state with the runtime, whose latched status can be
    /// stale after the app was suspended
    func refreshStatus() {
        rust.refreshStatus()
    }

    func apply(_ message: TorManagerReconcileMessage) {
        switch message {
        case let .configChanged(config):
            self.config = config
            connectionTestStates = [:]
            connectionTestError = nil
            autoStartSuppressed = false

            switch config {
            case .off:
                status = .off
                builtInFailure = nil

            case .builtIn:
                status = .bootstrapping(percent: 0, message: "Starting Tor")

            case .external:
                status = .stopped
                builtInFailure = nil
            }

        case let .bootstrapProgress(percent, message):
            status = .bootstrapping(percent: percent, message: message)
            builtInFailure = nil
            autoStartSuppressed = false

        case .ready:
            status = .ready
            builtInFailure = nil
            autoStartSuppressed = false

        case .stopped:
            // a stopped runtime without a tripped breaker means nothing is failing or suppressed
            status = config == .off ? .off : .stopped
            builtInFailure = nil
            autoStartSuppressed = false

        case .autoStartSuppressed:
            // the runtime was never launched, so nothing is bootstrapping behind this state
            status = .stopped
            autoStartSuppressed = true

        case let .failed(origin, error):
            let message = error.description

            switch origin {
            // the configured route may still be healthy, so leave the status alone
            case .connectionTest:
                connectionTestError = message

            // the route is untouched and the dispatch that asked for the fallback
            // reports this failure itself, so it must not look like a Tor outage
            case .lifecycle where error == .ClearnetFallback:
                break

            case .lifecycle:
                status = .failed(message: message)

                if config == .builtIn {
                    builtInFailure = TaggedString(message)
                }
            }

        case let .connectionTest(update):
            connectionTestStates[update.step] = update.state
        }
    }
}

private extension TorManagerDispatchResult {
    var disableWarning: TorDisableWarning? {
        switch self {
        case .applied:
            nil
        case let .disableWarning(warning):
            warning
        }
    }
}
