import SwiftUI

extension WeakReconciler: WalletManagerReconciler where Reconciler == WalletManager {}

@Observable class WalletManager: AnyReconciler, WalletManagerReconciler {
    private let logger = Log(id: "WalletManager")

    let id: WalletId
    var rust: RustWalletManager
    var walletMetadata: WalletMetadata
    var loadState: WalletLoadState = .loading
    var balance: Balance = .zero()
    var fiatBalance: Double?
    var foundAddresses: [FoundAddress] = []
    var unsignedTransactions: [UnsignedTransaction] = []

    // general wallet errors
    var errorAlert: WalletErrorAlert? = nil

    // errors in SendFlow
    var sendFlowErrorAlert: TaggedItem<SendFlowErrorAlert>? = nil

    public init(id: WalletId) throws {
        self.id = id
        let rust = try RustWalletManager(id: id)

        self.rust = rust

        walletMetadata = rust.walletMetadata()
        unsignedTransactions = (try? rust.getUnsignedTransactions()) ?? []

        rust.listenForUpdates(reconciler: WeakReconciler(self))
        Task {
            await updateFiatBalance()
        }
    }

    public init(xpub: String) throws {
        let rust = try RustWalletManager.tryNewFromXpub(xpub: xpub)
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

    func validateMetadata() {
        rust.validateMetadata()
    }

    func forceWalletScan() async {
        await rust.forceWalletScan()
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

    private func updateFiatBalance() async {
        do {
            let fiatBalance = try await rust.balanceInFiat()
            await MainActor.run {
                withAnimation { self.fiatBalance = fiatBalance }
            }
        } catch {
            Log.error("error getting fiat balance: \(error)")
            fiatBalance = 0.00
        }
    }

    func updateWalletBalance() async {
        let balance = await rust.balance()
        await MainActor.run { self.balance = balance }
        await updateFiatBalance()
    }

    func reconcile(message: WalletManagerReconcileMessage) {
        Task { [weak self] in
            guard let self else {
                Log.error("WalletManager no longer available")
                return
            }

            let rust = rust
            logger.debug("WalletManagerReconcileMessage: \(message)")

            await MainActor.run {
                switch message {
                case .startedInitialFullScan:
                    self.loadState = .loading

                case let .startedExpandedFullScan(txns):
                    self.loadState = .scanning(txns)

                case let .availableTransactions(txns):
                    if self.loadState == .loading {
                        self.loadState = .scanning(txns)
                    }

                case let .updatedTransactions(txns):
                    switch self.loadState {
                    case .scanning, .loading:
                        self.loadState = .scanning(txns)
                    case .loaded:
                        self.loadState = .loaded(txns)
                    }

                case let .scanComplete(txns):
                    self.loadState = .loaded(txns)

                case let .walletBalanceChanged(balance):
                    withAnimation { self.balance = balance }
                    Task { await self.updateFiatBalance() }

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

                case let .sendFlowError(error):
                    self.sendFlowErrorAlert = TaggedItem(error)
                }
            }
        }
    }

    public func dispatch(action: WalletManagerAction) {
        rust.dispatch(action: action)
    }

    // PREVIEW only
    public init(preview: String, _ walletMetadata: WalletMetadata? = nil) {
        assert(preview == "preview_only")

        id = WalletId()
        let rust =
            if let walletMetadata {
                RustWalletManager.previewNewWalletWithMetadata(metadata: walletMetadata)
            } else {
                RustWalletManager.previewNewWallet()
            }

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
