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
        _ = try await rust.dispatch(action: .runConnectionTest)
    }

    func apply(_ message: TorManagerReconcileMessage) {
        switch message {
        case let .configChanged(config):
            self.config = config
            connectionTestStates = [:]

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

        case .ready:
            status = .ready
            builtInFailure = nil

        case .stopped:
            status = config == .off ? .off : .stopped

        case let .failed(error):
            let message = error.description
            status = .failed(message: message)

            if config == .builtIn {
                builtInFailure = TaggedString(message)
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
