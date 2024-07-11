import SwiftUI

@Observable class WalletViewModel: WalletViewModelReconciler {
    private let logger = Log(id: "WalletViewModel")
    var rust: RustWalletViewModel
    var walletMetadata: WalletMetadata

    public init(id: WalletId) throws {
        let rust = try RustWalletViewModel(id: id)

        self.rust = rust
        walletMetadata = rust.walletMetadata()

        rust.listenForUpdates(reconciler: self)
    }

    var isVerified: Bool {
        walletMetadata.verified
    }

    func reconcile(message: WalletViewModelReconcileMessage) {
        Task {
            await MainActor.run {
                logger.debug("Reconcile: \(message)")

                switch message {
                case let .walletMetadataChanged(metadata):
                    walletMetadata = metadata
                }
            }
        }
    }

    public func dispatch(action: WalletViewModelAction) {
        rust.dispatch(action: action)
    }
}
