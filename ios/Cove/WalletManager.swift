import SwiftUI

extension WeakReconciler: WalletManagerReconciler where Reconciler == WalletManager {}

@Observable final class WalletManager: AnyReconciler, WalletManagerReconciler {
    typealias Message = WalletManagerReconcileMessage
    typealias Action = WalletManagerAction

    private let logger = Log(id: "WalletManager")

    let id: WalletId
    @ObservationIgnored
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

    // cached transaction details
    var transactionDetails: [TxId: TransactionDetails] = [:]

    public init(id: WalletId) throws {
        self.id = id
        let rust = try RustWalletManager(id: id)

        self.rust = rust

        walletMetadata = rust.walletMetadata()
        unsignedTransactions = (try? rust.getUnsignedTransactions()) ?? []

        updateFiatBalance()
        rust.listenForUpdates(reconciler: WeakReconciler(self))
    }

    public init(xpub: String) throws {
        let rust = try RustWalletManager.tryNewFromXpub(xpub: xpub)
        let metadata = rust.walletMetadata()

        self.rust = rust
        walletMetadata = metadata
        id = metadata.id

        updateFiatBalance()
        rust.listenForUpdates(reconciler: WeakReconciler(self))
    }

    public init(tapSigner: TapSigner, deriveInfo: DeriveInfo, backup: Data? = nil) throws {
        let rust = try RustWalletManager.tryNewFromTapSigner(
            tapSigner: tapSigner, deriveInfo: deriveInfo, backup: backup
        )

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

    var hasTransactions: Bool {
        switch loadState {
        case .loading: false
        case let .scanning(txns): !txns.isEmpty
        case let .loaded(txns): !txns.isEmpty
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

    func displayAmount(_ amount: Amount, showUnit: Bool = true) -> String {
        self.rust.displayAmount(amount: amount, showUnit: showUnit)
    }

    func amountFmtUnit(_ amount: Amount) -> String {
        switch walletMetadata.selectedUnit {
        case .btc: amount.btcStringWithUnit()
        case .sat: amount.satsStringWithUnit()
        }
    }

    func transactionDetails(for txId: TxId) async throws -> TransactionDetails {
        if let details = transactionDetails[txId] {
            return details
        }

        let details = try await rust.transactionDetails(txId: txId)
        transactionDetails[txId] = details

        return details
    }

    private func updateFiatBalance() {
        fiatBalance = rust.amountInFiat(amount: balance.spendable())
    }

    func updateWalletBalance() async {
        let balance = await rust.balance()
        await MainActor.run {
            self.balance = balance
            self.updateFiatBalance()
        }
    }

    func apply(_ message: Message) {
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
            updateFiatBalance()

        case .unsignedTransactionsChanged:
            self.unsignedTransactions = (try? rust.getUnsignedTransactions()) ?? []

        case let .walletMetadataChanged(metadata):
            withAnimation { self.walletMetadata = metadata }
            setWalletMetadata(metadata)

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

    private let rustBridge = DispatchQueue(
        label: "cove.walletmanager.rustbridge", qos: .userInitiated
    )

    private func setWalletMetadata(_ metadata: WalletMetadata) {
        rustBridge.async { [weak self] in
            self?.rust.setWalletMetadata(metadata: metadata)
        }
    }

    func reconcile(message: Message) {
        rustBridge.async { [weak self] in
            guard let self else {
                Log.error("WalletManager no longer available")
                return
            }

            logger.debug("reconcile: \(message)")
            DispatchQueue.main.async { [weak self] in
                self?.apply(message)
            }
        }
    }

    func reconcileMany(messages: [Message]) {
        rustBridge.async { [weak self] in
            guard let self else {
                Log.error("WalletManager no longer available")
                return
            }

            logger.debug("reconcile_messages: \(messages)")
            DispatchQueue.main.async { [weak self] in
                for message in messages {
                    self?.apply(message)
                }
            }
        }
    }

    public func dispatch(action: Action) { dispatch(action) }
    public func dispatch(_ action: Action) {
        rustBridge.async { [weak self] in
            self?.logger.debug("dispatch: \(action)")
            self?.rust.dispatch(action: action)
        }
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

    deinit {
        logger.debug("WalletManager deinit called for wallet \(id)")
    }
}

extension WalletLoadState: @retroactive Equatable {
    public static func == (lhs: WalletLoadState, rhs: WalletLoadState) -> Bool {
        walletStateIsEqual(lhs: lhs, rhs: rhs)
    }
}
