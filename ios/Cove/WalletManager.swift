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

private struct InitialScanLifecycleChangedHandler: @unchecked Sendable {
    let notify: () -> Void
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
    @ObservationIgnored
    private let initialScanLifecycleChanged =
        OSAllocatedUnfairLock<InitialScanLifecycleChangedHandler?>(initialState: nil)
    @ObservationIgnored
    private var walletScanStarted = false

    var walletMetadata: WalletMetadata
    var ledgerState: WalletLedgerState
    var loadState: WalletLoadState
    var scanStatus: WalletScanStatus
    var balancePresentation: BalancePresentation
    var balance: Balance = .zero()
    var foundAddresses: [FoundAddress] = []
    var unsignedTransactions: [UnsignedTransaction] = []

    var activeIncompleteInitialScan: Bool {
        // ledger activity and scan status arrive as separate reconcile messages
        ledgerState.initialScanActive || (ledgerState.initialScanIncomplete && scanStatus.isActive)
    }

    /// general wallet errors
    var errorAlert: WalletErrorAlert? = nil

    /// errors in SendFlow
    var sendFlowErrorAlert: TaggedItem<SendFlowErrorAlert>? = nil

    /// non-nil when a payjoin transaction has been broadcast (success or fallback);
    /// UUID changes each time so onChange always fires even across multiple sends
    var payjoinTxBroadcast: UUID? = nil

    /// cached transaction details
    var transactionDetails: [TxId: TransactionDetails] = [:]
    var transactionConfirmations: [TxId: UInt32] = [:]

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
        let rust = try RustWalletManager(id: id)
        let initialState = rust.initialState()

        self.id = initialState.metadata.id
        self.rust = rust
        self.loadState = initialState.loadState
        self.scanStatus = initialState.scanStatus
        self.ledgerState = initialState.ledgerState
        self.balancePresentation = initialState.balancePresentation
        self.balance = initialState.balance
        walletMetadata = initialState.metadata
        unsignedTransactions = initialState.unsignedTransactions

        rust.listenForUpdates(reconciler: WeakReconciler(self))
    }

    func close() {
        guard markClosedIfNeeded() else { return }
        rust.shutdown()
    }

    func setInitialScanLifecycleChanged(_ notify: (() -> Void)?) {
        initialScanLifecycleChanged.withLock { handler in
            handler = notify.map(InitialScanLifecycleChangedHandler.init(notify:))
        }
    }

    private func notifyInitialScanLifecycleChanged() {
        let handler = initialScanLifecycleChanged.withLock { $0?.notify }
        handler?()
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
        let initialState = rust.initialState()

        self.rust = rust
        self.loadState = initialState.loadState
        self.scanStatus = initialState.scanStatus
        self.ledgerState = initialState.ledgerState
        self.balancePresentation = initialState.balancePresentation
        self.balance = initialState.balance
        walletMetadata = initialState.metadata
        unsignedTransactions = initialState.unsignedTransactions
        id = initialState.metadata.id

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
        let initialState = rust.initialState()

        self.rust = rust
        self.loadState = initialState.loadState
        self.scanStatus = initialState.scanStatus
        self.ledgerState = initialState.ledgerState
        self.balancePresentation = initialState.balancePresentation
        self.balance = initialState.balance
        walletMetadata = initialState.metadata
        unsignedTransactions = initialState.unsignedTransactions
        id = initialState.metadata.id

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

    @MainActor
    func startWalletScanIfNeeded() async throws {
        guard !walletScanStarted else { return }
        walletScanStarted = true

        do {
            try await rust.startWalletScan()
        } catch {
            walletScanStarted = false
            throw error
        }
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
        walletDisplayAmount(metadata: walletMetadata, amount: amount, showUnit: showUnit)
    }

    func displayAmountPendingFmt(_ amount: Amount) -> String? {
        walletDisplayAmountPendingFmt(metadata: walletMetadata, amount: amount)
    }

    func displayAmountWithDirection(
        _ amount: Amount,
        direction: TransactionDirection
    ) -> String {
        walletDisplayAmountWithDirection(
            metadata: walletMetadata,
            amount: amount,
            direction: direction
        )
    }

    func displaySentAndReceivedAmount(_ sentAndReceived: SentAndReceived) -> String {
        walletDisplaySentAndReceivedAmount(
            metadata: walletMetadata,
            sentAndReceived: sentAndReceived
        )
    }

    func displayFiatAmount(_ amount: Double, withSuffix: Bool = true) -> String {
        walletDisplayFiatAmount(
            metadata: walletMetadata,
            amount: amount,
            withSuffix: withSuffix
        )
    }

    func displayFiatAmountPendingFmt(
        _ amount: Double,
        withSuffix: Bool = true
    ) -> String? {
        walletDisplayFiatAmountPendingFmt(
            metadata: walletMetadata,
            amount: amount,
            withSuffix: withSuffix
        )
    }

    func displayFiatAmountWithDirection(
        _ amount: Double,
        direction: TransactionDirection,
        withSuffix: Bool = true
    ) -> String {
        walletDisplayFiatAmountWithDirection(
            metadata: walletMetadata,
            amount: amount,
            direction: direction,
            withSuffix: withSuffix
        )
    }

    func amountInFiatCached(_ amount: Amount) -> Double? {
        walletAmountInFiatCached(amount: amount)
    }

    func amountFmtUnit(_ amount: Amount) -> String {
        switch walletMetadata.selectedUnit {
        case .btc: amount.btcStringWithUnit()
        case .sat: amount.satsStringWithUnit()
        }
    }

    func transactionDetails(for txId: TxId) async throws -> TransactionDetails {
        if let details = await MainActor.run(body: { transactionDetails[txId] }) {
            return details
        }

        let details = try await rust.transactionDetails(txId: txId)
        await MainActor.run {
            transactionDetails[txId] = details
        }

        return details
    }

    func refreshTransactionDetails(for txId: TxId) async throws -> TransactionDetails {
        let details = try await rust.transactionDetails(txId: txId)
        let confirmations: UInt32? = if let blockNumber = details.blockNumber() {
            try await rust.numberOfConfirmations(blockHeight: blockNumber)
        } else {
            nil
        }

        await MainActor.run {
            transactionDetails[txId] = details
            if let confirmations {
                transactionConfirmations[txId] = confirmations
            }
        }

        return details
    }

    func transactionLockState(for txId: TxId) async throws -> TransactionLockState {
        try await rust.transactionLockState(txId: txId)
    }

    func toggleTransactionLockState(for txId: TxId) async throws -> TransactionLockState {
        let state = try await rust.toggleTransactionLockState(txId: txId)
        await AppManager.shared.reconcileAfterLabelsChanged(walletId: id)

        return state
    }

    @MainActor
    func importLabels(labels: Bip329Labels) throws {
        try LabelManager(id: id).import(labels: labels)
        AppManager.shared.reconcileAfterLabelsChanged(walletId: id)
    }

    func reconcileAfterLabelsChanged() {
        let cachedTransactionIds = Array(transactionDetails.keys)

        Task {
            for txId in cachedTransactionIds {
                do {
                    _ = try await refreshTransactionDetails(for: txId)
                } catch {
                    logger.error("Failed to refresh transaction details after label change: \(error)")
                }
            }

            await rust.getTransactions()
        }
    }

    func updateTransactionConfirmations(txId: TxId, confirmations: UInt32) {
        transactionConfirmations[txId] = confirmations
    }

    private func replaceTransactionInLoadState(_ transaction: CoveCore.Transaction) {
        func replace(in txns: [CoveCore.Transaction]) -> [CoveCore.Transaction] {
            let txId = transaction.id
            var replaced = false
            let updated = txns.map { current in
                guard current.id == txId else { return current }
                replaced = true
                return transaction
            }

            return replaced ? updated : [transaction] + updated
        }

        switch loadState {
        case .loading:
            loadState = ledgerState.initialScanComplete ? .loaded([transaction]) : .scanning([transaction])
        case let .scanning(txns):
            loadState = .scanning(replace(in: txns))
        case let .loaded(txns):
            loadState = .loaded(replace(in: txns))
        }
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
            notifyInitialScanLifecycleChanged()

        case let .ledgerStateChanged(ledgerState):
            self.ledgerState = ledgerState
            self.balancePresentation = rust.balancePresentationForState(ledgerState: ledgerState)
            reconcileLoadStateWithLedgerState()
            notifyInitialScanLifecycleChanged()

        case let .availableTransactions(txns):
            switch self.loadState {
            case .loading:
                self.loadState = loadStateForTransactions(txns)
            case let .scanning(current) where txns.count >= current.count:
                self.loadState = loadStateForTransactions(txns)
            case .scanning:
                break
            case let .loaded(current) where txns.count >= current.count:
                self.loadState = loadStateForTransactions(txns)
            case .loaded:
                break
            }

        case let .updatedTransactions(txns):
            self.loadState = loadStateForTransactions(txns)

        case let .transactionUpdated(transaction):
            replaceTransactionInLoadState(transaction)

        case let .transactionDetailsUpdated(details):
            transactionDetails[details.txId()] = details

        case let .transactionConfirmationsUpdated(update):
            transactionConfirmations[update.txId] = update.confirmations

        case let .scanComplete(txns):
            self.loadState = loadStateForTransactions(txns)
            notifyInitialScanLifecycleChanged()

        case let .walletBalanceChanged(balance):
            withAnimation { self.balance = balance }

        case .unsignedTransactionsChanged:
            do {
                self.unsignedTransactions = try rust.getUnsignedTransactions()
            } catch {
                logger.error(
                    "Unable to refresh unsigned transactions: \(error.localizedDescription)"
                )
                self.unsignedTransactions = []
            }

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

        case .payjoinTxBroadcast:
            self.payjoinTxBroadcast = UUID()
        }
    }

    private let rustBridge = DispatchQueue(
        label: "cove.walletmanager.rustbridge", qos: .userInitiated
    )

    private func reconcileLoadStateWithLedgerState() {
        switch loadState {
        case .loading:
            break
        case let .scanning(txns), let .loaded(txns):
            loadState = loadStateForTransactions(txns)
        }
    }

    private func loadStateForTransactions(_ transactions: [CoveCore.Transaction]) -> WalletLoadState {
        if scanStatus.isActive {
            return .scanning(transactions)
        }

        if ledgerState.initialScanComplete {
            return .loaded(transactions)
        }

        if transactions.isEmpty {
            return .loading
        }

        return .scanning(transactions)
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

        let rust =
            if let walletMetadata {
                RustWalletManager.previewNewWalletWithMetadata(metadata: walletMetadata)
            } else {
                RustWalletManager.previewNewWallet()
            }

        self.rust = rust
        let initialState = rust.initialState()
        self.loadState = initialState.loadState
        self.scanStatus = initialState.scanStatus
        self.ledgerState = initialState.ledgerState
        self.balancePresentation = initialState.balancePresentation
        self.balance = initialState.balance
        self.walletMetadata = initialState.metadata
        self.id = initialState.metadata.id
        self.unsignedTransactions = initialState.unsignedTransactions

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
