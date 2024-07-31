import SwiftUI

extension WeakReconciler: WalletViewModelReconciler where Reconciler == WalletViewModel {}

@Observable class WalletViewModel: AnyReconciler, WalletViewModelReconciler {
    private let logger = Log(id: "WalletViewModel")

    let id: WalletId
    var rust: RustWalletViewModel
    var walletMetadata: WalletMetadata
    var loadState: WalletLoadState = .loading
    var balance: Balance = .init()

    public init(id: WalletId) throws {
        self.id = id
        let rust = try RustWalletViewModel(id: id)

        self.rust = rust
        walletMetadata = rust.walletMetadata()

        rust.listenForUpdates(reconciler: WeakReconciler(self))
    }

    var isVerified: Bool {
        walletMetadata.verified
    }

    func reconcile(message: WalletViewModelReconcileMessage) {
        Task {
            await MainActor.run {
                logger.debug("Reconcile: \(message)")

                switch message {
                case .startedWalletScan:
                    loadState = .loading

                case let .availableTransactions(txns):
                    loadState = .scanning(txns)

                case let .scanComplete(txns):
                    loadState = .loaded(txns)

                case .walletBalanceChanged:
                    Task {
                        let balance = await rust.balance()
                        await MainActor.run {
                            self.balance = balance
                        }
                    }

                case let .walletMetadataChanged(metadata):
                    walletMetadata = metadata

                case let .nodeConnectionFailed(error):
                    logger.error(error)
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
