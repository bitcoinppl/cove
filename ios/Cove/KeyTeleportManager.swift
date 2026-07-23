import os
import SwiftUI

extension WeakReconciler: KeyTeleportManagerReconciler where Reconciler == KeyTeleportManager {}

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

    func isSendEligible(walletId: WalletId) -> Bool {
        rust?.isSendEligible(walletId: walletId) ?? false
    }
}
