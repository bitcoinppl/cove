import os
import SwiftUI

extension WeakReconciler: KeyTeleportManagerReconciler where Reconciler == KeyTeleportManager {}

enum KeyTeleportFlowDirection: Equatable {
    case receive
    case send
}

enum KeyTeleportPacketRoutingDecision: Equatable {
    case accept
    case rejectConflict

    static func resolve(
        activeDirections: [KeyTeleportFlowDirection],
        requiredDirection: KeyTeleportFlowDirection
    ) -> Self {
        activeDirections.allSatisfy { $0 == requiredDirection } ? .accept : .rejectConflict
    }
}

enum KeyTeleportSendStartDecision: Equatable {
    case start
    case rejectActiveFlow

    static func resolve(activeDirections: [KeyTeleportFlowDirection]) -> Self {
        activeDirections.isEmpty ? .start : .rejectActiveFlow
    }
}

@Observable final class KeyTeleportManager: ReconcilingManager, KeyTeleportManagerReconciler {
    typealias Message = KeyTeleportManagerReconcileMessage
    typealias Action = KeyTeleportManagerAction

    private struct RustState {
        var rust: RustKeyTeleportManager?
        var isClosed = false
    }

    private let logger = Log(id: "KeyTeleportManager")
    @ObservationIgnored
    private let rustState = OSAllocatedUnfairLock(initialState: RustState())
    @ObservationIgnored
    private let rustBridge = DispatchQueue(label: "cove.KeyTeleportManager.rustbridge", qos: .userInitiated)

    private(set) var state: KeyTeleportManagerState
    var alert: KeyTeleportAlert?

    private var rust: RustKeyTeleportManager? {
        rustState.withLock { $0.rust }
    }

    init(_ rust: RustKeyTeleportManager) {
        state = rust.state()

        rustState.withLock { $0.rust = rust }
        rust.listenForUpdates(reconciler: WeakReconciler(self))
    }

    deinit {
        close()
    }

    func close() {
        guard let rust = takeRustForClose() else { return }

        logger.debug("Closing KeyTeleportManager")
        state = .idle
        alert = nil

        rustBridge.async {
            rust.dispatch(action: .clear)
        }
    }

    private func takeRustForClose() -> RustKeyTeleportManager? {
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
        case let .updateState(state):
            withAnimation { self.state = state }
        case let .setAlert(alert):
            self.alert = alert
        case .clearAlert:
            alert = nil
        }
    }

    func logReconcile(message _: Message) {
        logger.debug("Reconciling KeyTeleport update")
    }

    func logReconcileMany(messages _: [Message]) {
        logger.debug("Reconciling KeyTeleport updates")
    }

    func dispatch(_ action: Action) {
        rustBridge.async { [weak self] in
            guard let self, let rust = self.rust else { return }

            self.logger.debug("Dispatching KeyTeleport action")
            rust.dispatch(action: action)
        }
    }

    func ingest(_ input: String) {
        dispatch(.ingest(.multiFormat(StringOrData(input))))
    }

    func ingest(_ packet: KeyTeleportReceiverPacket) {
        dispatch(.ingest(.receiver(packet)))
    }

    func ingest(_ packet: KeyTeleportSenderPacket) {
        dispatch(.ingest(.sender(packet)))
    }

    func revealMnemonicWords() -> [String] {
        rust?.revealMnemonicWords() ?? []
    }

    func revealXprv() -> String? {
        rust?.revealXprv()
    }

    func hideSensitiveReveals() {
        dispatch(.hideXprv)
    }

    func isSendEligible(walletId: WalletId) -> Bool {
        rust?.isSendEligible(walletId: walletId) ?? false
    }

    var flowDirection: KeyTeleportFlowDirection? {
        state.flowDirection
    }
}

extension KeyTeleportManagerState {
    var flowDirection: KeyTeleportFlowDirection? {
        switch self {
        case .idle:
            nil
        case .receiveReady,
             .receiveError,
             .receiveEnterPassword,
             .receiveMnemonicReview,
             .receiveXprvReview,
             .receiveMessageReview,
             .receiveImportedWallet,
             .receiveAlreadyImportedWallet:
            .receive
        case .sendAwaitReceiver,
             .sendChooseWallet,
             .sendEnterCode,
             .sendReady:
            .send
        }
    }
}

extension KeyTeleportRoute {
    var flowDirection: KeyTeleportFlowDirection {
        switch self {
        case .receive:
            .receive
        case .send:
            .send
        }
    }
}

extension Route {
    var keyTeleportFlowDirection: KeyTeleportFlowDirection? {
        guard case let .keyTeleport(route) = self else { return nil }
        return route.flowDirection
    }
}
