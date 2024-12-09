import SwiftUI

@Observable class ImportWalletManager: ImportWalletManagerReconciler {
    private let logger = Log(id: "ImportWalletManager")
    var rust: RustImportWalletManager

    public init() {
        rust = RustImportWalletManager()
        rust.listenForUpdates(reconciler: self)
    }

    func reconcile(message: ImportWalletManagerReconcileMessage) {
        Task {
            await MainActor.run {
                logger.debug("Reconcile: \(message)")

                switch message {
                case .noOp:
                    break
                }
            }
        }
    }

    public func dispatch(action: ImportWalletManagerAction) {
        logger.debug("Dispatch: \(action)")
        rust.dispatch(action: action)
    }
}
