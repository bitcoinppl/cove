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

    func reconcile(message: WalletViewModelReconcileMessage) {
        Task { [weak self] in
            guard let self = self else { return }
            let rust = self.rust

            self.logger.debug("WalletViewModelReconcileMessage: \(message)")

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

            case let .walletError(error):
                // TODO: show to user
                self.logger.error("WalletError \(error)")

            case let .unknownError(error):
                // TODO: show to user
                self.logger.error("Unknown error \(error)")
            }
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

extension WalletLoadState: Equatable {
    public static func == (lhs: WalletLoadState, rhs: WalletLoadState) -> Bool {
        walletStateIsEqual(lhs: lhs, rhs: rhs)
    }
}
