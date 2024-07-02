import SwiftUI

@Observable class WalletViewModel: WalletViewModelReconciler {
    var rust: RustWalletViewModel
    var walletMetadata: WalletMetadata

    public init(id: WalletId) throws {
        let rust = try RustWalletViewModel(id: id)

        self.rust = rust
        walletMetadata = rust.getState().walletMetadata

        rust.listenForUpdates(reconciler: self)
    }

    func reconcile(message: WalletViewModelReconcileMessage) {
        Task {
            await MainActor.run {
                print("[swift] WalletViewModel Reconcile: \(message)")

                switch message {
                case .noOp:
                    break
                }
            }
        }
    }

    public func dispatch(action: WalletViewModelAction) {
        rust.dispatch(action: action)
    }
}
