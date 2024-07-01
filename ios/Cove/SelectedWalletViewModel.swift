import SwiftUI

@Observable class SelectedWalletViewModel: SelectedWalletViewModelReconciler {
    var rust: RustSelectedWalletViewModel
    var walletMetadata: WalletMetadata

    public init(id: WalletId) throws {
        let rust = try RustSelectedWalletViewModel(id: id)

        self.rust = rust
        walletMetadata = rust.getState().walletMetadata

        rust.listenForUpdates(reconciler: self)
    }

    func reconcile(message: SelectedWalletViewModelReconcileMessage) {
        Task {
            await MainActor.run {
                print("[swift] SelectedWalletViewModel Reconcile: \(message)")

                switch message {
                case .noOp:
                    break
                }
            }
        }
    }

    public func dispatch(action: SelectedWalletViewModelAction) {
        rust.dispatch(action: action)
    }
}
