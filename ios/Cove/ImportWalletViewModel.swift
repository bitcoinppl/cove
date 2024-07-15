import SwiftUI

@Observable class ImportWalletViewModel: ImportWalletViewModelReconciler {
    private let logger = Log(id: "ImportWalletViewModel")
    var rust: RustImportWalletViewModel

    public init() {
        rust = RustImportWalletViewModel()
        rust.listenForUpdates(reconciler: self)
    }

    func reconcile(message: ImportWalletViewModelReconcileMessage) {
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

    public func dispatch(action: ImportWalletViewModelAction) {
        logger.debug("Dispatch: \(action)")
        rust.dispatch(action: action)
    }
}
