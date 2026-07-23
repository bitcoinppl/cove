import os
import SwiftUI

extension WeakReconciler: NodeScannerReconciler where Reconciler == NodeScannerManager {}

private enum NodeScannerManagerError: LocalizedError {
    case closed

    var errorDescription: String? {
        "Node scanner manager is closed"
    }
}

@Observable final class NodeScannerManager: ReconcilingManager, NodeScannerReconciler {
    typealias Message = NodeScannerReconcileMessage

    private struct RustState {
        var rust: RustNodeScannerManager?
        var isClosed = false
    }

    private let logger = Log(id: "NodeScannerManager")
    @ObservationIgnored
    private let rustState = OSAllocatedUnfairLock(initialState: RustState())
    @ObservationIgnored
    private let rustBridge = DispatchQueue(
        label: "cove.NodeScannerManager.rustbridge",
        qos: .userInitiated
    )
    @ObservationIgnored
    private var pendingRustOperation: Task<Void, Never>?

    private(set) var state: NodeScannerState = .idle

    private var rust: RustNodeScannerManager? {
        rustState.withLock { $0.rust }
    }

    init() {
        let rust = RustNodeScannerManager()
        rustState.withLock { $0.rust = rust }
        rust.listenForUpdates(reconciler: WeakReconciler(self))
        state = rust.state()
    }

    deinit {
        close()
    }

    func start() async {
        try? await performRustOperation { rust in
            await rust.startScan()
        }
    }

    func stop() async {
        try? await performRustOperation { rust in
            await rust.stopScan()
        }
    }

    func createNode(_ found: FoundNode) async throws -> Node {
        try await performRustOperation { rust in
            try await rust.createNode(found: found)
        }
    }

    func trustAndCreateNode(_ candidate: TrustRequiredEndpoint) async throws -> Node {
        try await performRustOperation { rust in
            try await rust.trustAndCreateNode(candidate: candidate)
        }
    }

    func close() {
        guard takeRustForClose() != nil else { return }

        logger.debug("Closing NodeScannerManager")
    }

    private func performRustOperation<T: Sendable>(
        _ operation: @escaping @Sendable (RustNodeScannerManager) async throws -> T
    ) async throws -> T {
        try await withCheckedThrowingContinuation { continuation in
            rustBridge.async { [weak self] in
                guard let self else {
                    continuation.resume(throwing: NodeScannerManagerError.closed)
                    return
                }

                let previous = self.pendingRustOperation
                let task = Task { [weak self] in
                    await previous?.value

                    guard let self, let rust = self.rust else {
                        continuation.resume(throwing: NodeScannerManagerError.closed)
                        return
                    }

                    do {
                        try await continuation.resume(returning: operation(rust))
                    } catch {
                        continuation.resume(throwing: error)
                    }
                }
                self.pendingRustOperation = task
            }
        }
    }

    private func takeRustForClose() -> RustNodeScannerManager? {
        rustState.withLock { state in
            guard !state.isClosed else { return nil }

            state.isClosed = true
            let rust = state.rust
            state.rust = nil
            return rust
        }
    }

    var canApplyReconcileMessages: Bool {
        rust != nil
    }

    func apply(_ message: Message) {
        switch message {
        case let .scanStarted(generation, network, progress):
            guard currentGeneration.map({ generation > $0 }) ?? true else { return }

            state = .scanning(
                generation: generation,
                network: network,
                progress: progress,
                results: []
            )

        case let .scanProgress(generation, progress):
            guard case let .scanning(currentGeneration, network, _, results) = state,
                  currentGeneration == generation
            else {
                return
            }

            state = .scanning(
                generation: generation,
                network: network,
                progress: progress,
                results: results
            )

        case let .resultUpserted(generation, result):
            guard let state = updatingResults(for: generation, with: { results in
                results = upserting(result, into: results)
            }) else { return }

            self.state = state

        case let .resultRemoved(generation, ip):
            guard let state = updatingResults(for: generation, with: { results in
                results.removeAll { resultIp($0) == ip }
            }) else { return }

            self.state = state

        case let .scanStopped(generation):
            guard case let .scanning(currentGeneration, network, _, results) = state,
                  currentGeneration == generation
            else {
                return
            }

            state = .stopped(generation: generation, network: network, results: results)

        case let .scanFinished(generation):
            guard case let .scanning(currentGeneration, network, _, results) = state,
                  currentGeneration == generation
            else {
                return
            }

            state = .finished(generation: generation, network: network, results: results)

        case let .scanFailed(generation, error):
            guard case let .scanning(currentGeneration, network, _, results) = state,
                  currentGeneration == generation
            else {
                return
            }

            state = .failed(
                generation: generation,
                network: network,
                error: error,
                results: results
            )
        }
    }

    func logReconcile(message: Message) {
        logger.debug("reconcile: \(message)")
    }

    func logReconcileMany(messages: [Message]) {
        logger.debug("reconcile_messages: \(messages)")
    }

    private var currentGeneration: UInt64? {
        switch state {
        case .idle:
            nil
        case let .scanning(generation, _, _, _),
             let .stopped(generation, _, _),
             let .finished(generation, _, _),
             let .failed(generation, _, _, _):
            generation
        }
    }

    private func upserting(
        _ result: NodeScannerResult,
        into results: [NodeScannerResult]
    ) -> [NodeScannerResult] {
        var results = results

        if let index = results.firstIndex(where: { resultIp($0) == resultIp(result) }) {
            results[index] = result
        } else {
            results.append(result)
        }

        return results
    }

    private func updatingResults(
        for generation: UInt64,
        with update: (inout [NodeScannerResult]) -> Void
    ) -> NodeScannerState? {
        guard currentGeneration == generation else { return nil }

        switch state {
        case .idle, .failed:
            return nil

        case let .scanning(_, network, progress, currentResults):
            var results = currentResults
            update(&results)
            return .scanning(
                generation: generation,
                network: network,
                progress: progress,
                results: results
            )

        case let .stopped(_, network, currentResults):
            var results = currentResults
            update(&results)
            return .stopped(generation: generation, network: network, results: results)

        case let .finished(_, network, currentResults):
            var results = currentResults
            update(&results)
            return .finished(generation: generation, network: network, results: results)
        }
    }

    private func resultIp(_ result: NodeScannerResult) -> String {
        switch result {
        case let .verified(node):
            node.ip
        case let .trustRequired(endpoint):
            endpoint.ip
        }
    }
}
