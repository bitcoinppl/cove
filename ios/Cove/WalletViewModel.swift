import SwiftUI

@Observable class WalletViewModel: WalletViewModelReconciler {
    private let logger = Log(id: "WalletViewModel")

    let id: WalletId
    var rust: RustWalletViewModel
    var walletMetadata: WalletMetadata
    var loadState: WalletLoadState = .loading

    public init(id: WalletId) throws {
        self.id = id
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
                case let .walletBalanceChanged(balance):
                    ()
                case .completedWalletScan:
                    ()
                case .startedWalletScan:
                    ()
                }
            }
        }
    }

    public func dispatch(action: WalletViewModelAction) {
        rust.dispatch(action: action)
    }

    // PREVIEW only
    public init(preview: String) {
        assert(preview == "preview_only")

        id = WalletId()
        let rust = RustWalletViewModel.previewNewWallet()

        self.rust = rust
        walletMetadata = rust.walletMetadata()

        rust.listenForUpdates(reconciler: self)
    }
}
