import SwiftUI

extension WeakReconciler: WalletViewModelReconciler where Reconciler == WalletViewModel {}

@Observable class WalletViewModel: AnyReconciler, WalletViewModelReconciler {
    private let logger = Log(id: "WalletViewModel")

    let id: WalletId
    var rust: RustWalletViewModel
    var walletMetadata: WalletMetadata
    var loadState: WalletLoadState = .loading
    var balance: Balance = .init()
    var errorAlert: WalletErrorAlert? = nil
    var foundAddresses: [FoundAddress] = []

    public init(id: WalletId) throws {
        self.id = id
        let rust = try RustWalletViewModel(id: id)

        self.rust = rust
        walletMetadata = rust.walletMetadata()

        rust.listenForUpdates(reconciler: WeakReconciler(self))
    }

    public init(xpub: String) throws {
        let rust = try RustWalletViewModel.tryNewFromXpub(xpub: xpub)
        let metadata = rust.walletMetadata()

        self.rust = rust
        walletMetadata = metadata
        id = metadata.id

        rust.listenForUpdates(reconciler: WeakReconciler(self))
    }

    var unit: String {
        return switch walletMetadata.selectedUnit {
        case .btc: "btc"
        case .sat: "sats"
        }
    }

    var isVerified: Bool {
        walletMetadata.verified
    }

    var accentColor: Color {
        walletMetadata.swiftColor
    }

    func firstAddress() async throws -> AddressInfo {
        return try await rust.addressAt(index: 0)
    }

    func amountFmt(_ amount: Amount) -> String {
        switch walletMetadata.selectedUnit {
        case .btc:
            return amount.btcString()
        case .sat:
            return amount.satsString()
        }
    }

    func amountFmtUnit(_ amount: Amount) -> String {
        switch walletMetadata.selectedUnit {
        case .btc:
            return amount.btcStringWithUnit()
        case .sat:
            return amount.satsStringWithUnit()
        }
    }

    func fiatAmountToString<T: Numeric & LosslessStringConvertible>(_ amount: T) -> String {
        "≈ \(FiatFormatter(amount).fmt()) USD"
    }

    func reconcile(message: WalletViewModelReconcileMessage) {
        Task { [weak self] in
            guard let self = self else {
                Log.error("WalletViewModel no longer available")
                return
            }

            let rust = self.rust
            self.logger.debug("WalletViewModelReconcileMessage: \(message)")

            await MainActor.run {
                switch message {
                case .startedWalletScan:
                    self.loadState = .loading

                case let .availableTransactions(txns):
                    if self.loadState == .loading {
                        self.loadState = .scanning(txns)
                    }

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
                    withAnimation {
                        self.walletMetadata = metadata
                        self.rust.setWalletMetadata(metadata: metadata)
                    }

                case let .walletScannerResponse(scannerResponse):
                    self.logger.debug("walletScannerResponse: \(scannerResponse)")
                    if case let .foundAddresses(addressTypes) = scannerResponse {
                        self.foundAddresses = addressTypes
                    }

                case let .nodeConnectionFailed(error):
                    self.errorAlert = WalletErrorAlert.nodeConnectionFailed(error)
                    self.logger.error(error)
                    self.logger.error("set errorAlert")

                case let .walletError(error):
                    self.logger.error("WalletError \(error)")

                case let .unknownError(error):
                    // TODO: show to user
                    self.logger.error("Unknown error \(error)")
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

        rust.listenForUpdates(reconciler: WeakReconciler(self))
    }
}

extension WalletLoadState: Equatable {
    public static func == (lhs: WalletLoadState, rhs: WalletLoadState) -> Bool {
        walletStateIsEqual(lhs: lhs, rhs: rhs)
    }
}
