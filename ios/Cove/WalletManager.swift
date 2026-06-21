import os
import SwiftUI

extension WeakReconciler: WalletManagerReconciler where Reconciler == WalletManager {}

extension WalletScanStatus {
    var isActive: Bool {
        // internal visibility lets wallet screens share the same active-scan definition
        switch self {
        case .idle:
            false
        case .scanning, .scanningPendingProgress:
            true
        }
    }
}

extension WalletLedgerState {
    var initialScanComplete: Bool {
        if case .complete = self {
            return true
        }

        return false
    }

    var initialScanIncomplete: Bool {
        !initialScanComplete
    }

    var initialScanActive: Bool {
        if case .initialScanIncomplete(.active) = self {
            return true
        }

        return false
    }
}

@Observable final class WalletManager: AnyReconciler, WalletManagerReconciler {
    typealias Message = WalletManagerReconcileMessage
    typealias Action = WalletManagerAction

    private let logger = Log(id: "WalletManager")

    let id: WalletId
    @ObservationIgnored
    let rust: RustWalletManager
    @ObservationIgnored
    private let closeState = OSAllocatedUnfairLock(initialState: false)

    var walletMetadata: WalletMetadata
    var ledgerState: WalletLedgerState
    var loadState: WalletLoadState
    var scanStatus: WalletScanStatus
    var balancePresentation: BalancePresentation
    var balance: Balance = .zero()
    var foundAddresses: [FoundAddress] = []
    var unsignedTransactions: [UnsignedTransaction] = []

    /// general wallet errors
    var errorAlert: WalletErrorAlert? = nil

    /// errors in SendFlow
    var sendFlowErrorAlert: TaggedItem<SendFlowErrorAlert>? = nil

    /// cached transaction details
    var transactionDetails: [TxId: TransactionDetails] = [:]

    var receiveAddressState: ReceiveAddressState?
    var receiveAddressPresentation = ReceiveAddressPresentation(
        copyPolicy: .copy,
        refreshState: .idle
    )
    var receiveAddressIsLoading = false
    var receiveAddressError: TaggedString?

    /// scroll position for transaction list (persists across navigation)
    var scrolledTransactionId: String?

    init(id: WalletId) throws {
        self.id = id
        let rust = try RustWalletManager(id: id)
        let loadState = rust.initialLoadState()

        self.rust = rust
        self.loadState = loadState
        self.scanStatus = .idle
        self.ledgerState = rust.ledgerState()
        self.balancePresentation = rust.balancePresentation(scanStatus: .idle)

        walletMetadata = rust.walletMetadata()
        unsignedTransactions = (try? rust.getUnsignedTransactions()) ?? []

        rust.listenForUpdates(reconciler: WeakReconciler(self))
    }

    func close() {
        guard markClosedIfNeeded() else { return }
        rust.shutdown()
    }

    private func markClosedIfNeeded() -> Bool {
        closeState.withLock { isClosed in
            guard !isClosed else { return false }
            isClosed = true
            return true
        }
    }

    init(xpub: String) throws {
        let rust = try RustWalletManager.tryNewFromXpub(xpub: xpub)
        let metadata = rust.walletMetadata()

        self.rust = rust
        self.loadState = .loading
        self.scanStatus = .idle
        self.ledgerState = rust.ledgerState()
        self.balancePresentation = rust.balancePresentation(scanStatus: .idle)
        walletMetadata = metadata
        id = metadata.id

        rust.listenForUpdates(reconciler: WeakReconciler(self))
    }

    init(
        tapSigner: TapSigner,
        deriveInfo: DeriveInfo,
        backup: Data? = nil,
        birthday: WalletBirthday? = nil
    ) throws {
        let rust = try RustWalletManager.tryNewFromTapSigner(
            tapSigner: tapSigner,
            deriveInfo: deriveInfo,
            backup: backup,
            birthday: birthday
        )

        let metadata = rust.walletMetadata()

        self.rust = rust
        self.loadState = .loading
        self.scanStatus = .idle
        self.ledgerState = rust.ledgerState()
        self.balancePresentation = rust.balancePresentation(scanStatus: .idle)
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

    func updateTransactionDetailsCache(txId: TxId, details: TransactionDetails) {
        transactionDetails[txId] = details
    }

    func updateWalletBalance() async {
        let balance = await rust.balance()
        await MainActor.run {
            self.balance = balance
        }
    }

    func apply(_ message: Message) {
        switch message {
        case let .walletScanStatusChanged(status):
            self.scanStatus = status
            self.balancePresentation = rust.balancePresentationForState(ledgerState: ledgerState)
            if status.isActive {
                switch self.loadState {
                case .scanning:
                    break
                case let .loaded(txns):
                    self.loadState = .scanning(txns)
                case .loading:
                    self.loadState = .scanning([])
                }
            } else if case let .scanning(txns) = self.loadState {
                if ledgerState.initialScanComplete {
                    self.loadState = .loaded(txns)
                }
            }

        case let .ledgerStateChanged(ledgerState):
            self.ledgerState = ledgerState
            self.balancePresentation = rust.balancePresentationForState(ledgerState: ledgerState)
            reconcileLoadStateWithLedgerState()

        case let .availableTransactions(txns):
            switch self.loadState {
            case .loading:
                self.loadState = .scanning(txns)
            case let .scanning(current) where txns.count >= current.count:
                self.loadState = .scanning(txns)
            case .scanning:
                break
            case let .loaded(current) where txns.count >= current.count:
                self.loadState = .scanning(txns)
            case .loaded:
                break
            }

        case let .updatedTransactions(txns):
            switch self.loadState {
            case .scanning, .loading:
                self.loadState = .scanning(txns)
            case .loaded:
                self.loadState = ledgerState.initialScanComplete
                    ? .loaded(txns)
                    : .scanning(txns)
            }

        case let .scanComplete(txns):
            self.loadState = ledgerState.initialScanComplete ? .loaded(txns) : .scanning(txns)

        case let .walletBalanceChanged(balance):
            withAnimation { self.balance = balance }

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

        case let .hotWalletKeyMissing(walletId):
            AppManager.shared.alertState = .init(.hotWalletKeyMissing(walletId: walletId))

        case let .receiveAddressUpdated(state):
            self.receiveAddressState = state

        case let .receiveAddressPresentationUpdated(presentation):
            self.receiveAddressPresentation = presentation

        case let .receiveAddressLoadingChanged(isLoading):
            self.receiveAddressIsLoading = isLoading

        case let .receiveAddressError(error):
            self.receiveAddressError = TaggedString(error)

        case let .receiveAddressClosed(requestId):
            if receiveAddressState?.requestId == requestId {
                receiveAddressState = nil
                receiveAddressPresentation = ReceiveAddressPresentation(
                    copyPolicy: .copy,
                    refreshState: .idle
                )
                receiveAddressIsLoading = false
                receiveAddressError = nil
            }
        }
    }

    private let rustBridge = DispatchQueue(
        label: "cove.walletmanager.rustbridge", qos: .userInitiated
    )

    private func reconcileLoadStateWithLedgerState() {
        if ledgerState.initialScanComplete, !scanStatus.isActive,
           case let .scanning(txns) = loadState
        {
            loadState = .loaded(txns)
        } else if ledgerState.initialScanIncomplete,
                  scanStatus.isActive,
                  case let .loaded(txns) = loadState
        {
            loadState = .scanning(txns)
        }
    }

    private func setWalletMetadata(_ metadata: WalletMetadata) {
        rustBridge.async { [weak self] in
            self?.rust.setWalletMetadata(metadata: metadata)
        }
    }

    func reconcile(message: Message) {
        DispatchQueue.main.async { [weak self] in
            guard let self else { return }
            logger.debug("reconcile \(message)")
            self.apply(message)
        }
    }

    func reconcileMany(messages: [Message]) {
        DispatchQueue.main.async { [weak self] in
            guard let self else { return }
            logger.debug("reconcile_messages: \(messages)")
            messages.forEach { self.apply($0) }
        }
    }

    func dispatch(action: Action) {
        dispatch(action)
    }

    func dispatch(_ action: Action) {
        if case .openReceiveAddress = action {
            receiveAddressError = nil
        }

        if case .createNewReceiveAddress = action {
            receiveAddressError = nil
        }

        rustBridge.async { [weak self] in
            self?.logger.debug("dispatch: \(action)")
            self?.rust.dispatch(action: action)
        }
    }

    /// PREVIEW only
    init(preview: String, _ walletMetadata: WalletMetadata? = nil) {
        assert(preview == "preview_only")

        id = WalletId()
        let rust =
            if let walletMetadata {
                RustWalletManager.previewNewWalletWithMetadata(metadata: walletMetadata)
            } else {
                RustWalletManager.previewNewWallet()
            }

        self.rust = rust
        self.loadState = .loading
        self.scanStatus = .idle
        self.ledgerState = rust.ledgerState()
        self.balancePresentation = rust.balancePresentation(scanStatus: .idle)
        self.walletMetadata = rust.walletMetadata()

        rust.listenForUpdates(reconciler: WeakReconciler(self))
    }

    deinit {
        close()
        logger.debug("WalletManager deinit called for wallet \(id)")
    }
}

extension WalletLoadState: @retroactive Equatable {
    public static func == (lhs: WalletLoadState, rhs: WalletLoadState) -> Bool {
        lhs.isEqual(other: rhs)
    }
}
