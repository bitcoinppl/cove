import SwiftUI

extension WeakReconciler: WalletViewModelReconciler where Reconciler == WalletViewModel {}

@Observable class WalletViewModel: AnyReconciler, WalletViewModelReconciler {
    private let logger = Log(id: "WalletViewModel")

    let id: WalletId
    var rust: RustWalletViewModel
    var walletMetadata: WalletMetadata
    var loadState: WalletLoadState = .loading
    var balance: Balance = .init()
    var fiatBalance: Double?
    var errorAlert: WalletErrorAlert? = nil
    var foundAddresses: [FoundAddress] = []
    var unsignedTransactions: [UnsignedTransaction] = []

    public init(id: WalletId) throws {
        self.id = id
        let rust = try RustWalletViewModel(id: id)

        self.rust = rust

        walletMetadata = rust.walletMetadata()
        unsignedTransactions = (try? rust.getUnsignedTransactions()) ?? []

        rust.listenForUpdates(reconciler: WeakReconciler(self))
        Task {
            await getFiatBalance()
        }
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
        switch walletMetadata.selectedUnit {
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
        try await rust.addressAt(index: 0)
    }

    func amountFmt(_ amount: Amount) -> String {
        switch walletMetadata.selectedUnit {
        case .btc:
            amount.btcString()
        case .sat:
            amount.satsString()
        }
    }

    func amountFmtUnit(_ amount: Amount) -> String {
        switch walletMetadata.selectedUnit {
        case .btc: amount.btcStringWithUnit()
        case .sat: amount.satsStringWithUnit()
        }
    }

    func fiatAmountToString(_ amount: some Numeric & LosslessStringConvertible) -> String {
        "≈ \(FiatFormatter(amount).fmt()) USD"
    }

    func getFiatBalance() async {
        do {
            let fiatBalance = try await rust.balanceInFiat()
            await MainActor.run { self.fiatBalance = fiatBalance }
        } catch {
            Log.error("error getting fiat balance: \(error)")
            fiatBalance = 0.00
        }
    }

    func reconcile(message: WalletViewModelReconcileMessage) {
        Task { [weak self] in
            guard let self else {
                Log.error("WalletViewModel no longer available")
                return
            }

            let rust = rust
            logger.debug("WalletViewModelReconcileMessage: \(message)")

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

                case .unsignedTransactionsChanged:
                    self.unsignedTransactions = (try? rust.getUnsignedTransactions()) ?? []

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
