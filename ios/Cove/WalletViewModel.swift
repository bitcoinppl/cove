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
        self.walletMetadata = rust.walletMetadata()

        rust.listenForUpdates(reconciler: WeakReconciler(self))
    }

    var isVerified: Bool {
        self.walletMetadata.verified
    }

    @MainActor
    func reconcile(message: WalletViewModelReconcileMessage) {
        let rust = self.rust

        self.logger.debug("Reconcile: \(message)")

        switch message {
        case .startedWalletScan:
            self.loadState = .loading

        case let .availableTransactions(txns):
            self.loadState = .scanning(txns)

        case let .scanComplete(txns):
            self.loadState = .loaded(txns)

        case .walletBalanceChanged:
            Task {
                let balance = await rust.balance()
                await MainActor.run {
                    self.balance = balance
                }
            }

        case let .walletMetadataChanged(metadata):
            self.walletMetadata = metadata

        case let .nodeConnectionFailed(error):
            self.logger.error(error)
        }
    }

    public func dispatch(action: WalletViewModelAction) {
        self.rust.dispatch(action: action)
    }

    // PREVIEW only
    public init(preview: String) {
        assert(preview == "preview_only")

        self.id = WalletId()
        let rust = RustWalletViewModel.previewNewWallet()

        self.rust = rust
        self.walletMetadata = rust.walletMetadata()

        rust.listenForUpdates(reconciler: WeakReconciler(self))
    }
}
