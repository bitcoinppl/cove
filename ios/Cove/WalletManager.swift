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

private struct WalletManagerBootstrap {
    let rust: RustWalletManager
    let initialState: WalletInitialState
}

private enum WalletManagerAccessError: LocalizedError {
    case closed

    var errorDescription: String? {
        "Wallet manager is closed"
    }
}

enum WalletAddressTypeSwitchResult: Equatable {
    case committed
    case committedWithRecoveryPending
}

func walletManagerIsClosing(_ error: Error, walletId: WalletId) -> Bool {
    guard case let WalletManagerError.WalletLifecycle(.managerClosing(closingId)) = error else {
        return false
    }

    return closingId == walletId
}

func walletManagerConstructionIsInProgress(_ error: Error, walletId: WalletId) -> Bool {
    guard case let WalletManagerError.WalletLifecycle(
        .constructionInProgress(constructionId)
    ) = error else {
        return false
    }

    return constructionId == walletId
}

func committedAddressTypeSwitchResult(
    _ error: Error,
    requestedType: WalletAddressType
) -> WalletAddressTypeSwitchResult? {
    guard case let WalletManagerError.AddressTypeSwitchCommittedWithRecoveryPending(
        addressType,
        _
    ) = error, addressType == requestedType else {
        return nil
    }

    return .committedWithRecoveryPending
}

enum WalletManagerPreview {
    case only
}

@Observable final class WalletManager: ReconcilingManager, WalletManagerReconciler {
    typealias Message = WalletManagerReconcileMessage
    typealias Action = WalletManagerAction

    private let logger = Log(id: "WalletManager")

    private struct RustState {
        var rust: RustWalletManager? = nil
        var isClosed = false
    }

    let id: WalletId
    @ObservationIgnored
    private let rustState = OSAllocatedUnfairLock(initialState: RustState())
    @ObservationIgnored
    private let initialScanLifecycleChanged =
        OSAllocatedUnfairLock<InitialScanLifecycleChangedHandler?>(initialState: nil)
    @ObservationIgnored
    private var walletScanStarted = false
    @ObservationIgnored
    private weak var delegate: WalletManagerDelegate?

    private var rust: RustWalletManager? {
        rustState.withLock { $0.rust }
    }

    var walletMetadata: WalletMetadata
    var ledgerState: WalletLedgerState
    var loadState: WalletLoadState
    var scanStatus: WalletScanStatus
    var balancePresentation: BalancePresentation
    var balance: Balance = .zero()
    var foundAddresses: [FoundAddress] = []
    var unsignedTransactions: [UnsignedTransaction] = []

    func hasRecoveryWords() -> Bool {
        withRustOr(false) { try $0.hasRecoveryWords() }
    }

    func hasXprvSecret() -> Bool {
        withRustOr(false) { try $0.hasXprvSecret() }
    }

    func switchToDifferentWalletAddressType(
        _ addressType: WalletAddressType
    ) async throws -> WalletAddressTypeSwitchResult {
        do {
            try await withRustAsync {
                try await $0.switchToDifferentWalletAddressType(walletAddressType: addressType)
            }
            return .committed
        } catch {
            guard let result = committedAddressTypeSwitchResult(
                error,
                requestedType: addressType
            ) else {
                throw error
            }

            logger.error("Wallet address type switched with recovery pending: \(error)")
            return result
        }
    }

    var activeIncompleteInitialScan: Bool {
        // ledger activity and scan status arrive as separate reconcile messages
        ledgerState.initialScanActive || (ledgerState.initialScanIncomplete && scanStatus.isActive)
    }

    /// general wallet errors
    var errorAlert: TaggedItem<WalletErrorAlert>? = nil

    /// errors in SendFlow
    var sendFlowErrorAlert: TaggedItem<SendFlowErrorAlert>? = nil

    /// non-nil when a payjoin transaction has been broadcast (success or fallback);
    /// UUID changes each time so onChange always fires even across multiple sends
    var payjoinTxBroadcast: UUID? = nil

    /// epoch seconds when the payjoin session will expire, set when polling starts
    var payjoinDeadlineSecs: UInt64? = nil

    /// cached transaction detail presentations
    var transactionDetailsPresentations: [TxId: TransactionDetailsPresentation] = [:]
    var transactionLockStates: [TxId: TransactionLockState] = [:]

    var receiveAddressState: ReceiveAddressState?
    var receiveAddressPresentation = ReceiveAddressPresentation(
        copyPolicy: .copy,
        refreshState: .idle
    )
    var receiveAddressIsLoading = false
    var receiveAddressError: TaggedString?

    /// scroll position for transaction list (persists across navigation)
    var scrolledTransactionId: String?

    private init(
        rust: RustWalletManager,
        initialState: WalletInitialState,
        delegate: WalletManagerDelegate?
    ) {
        self.id = initialState.metadata.id
        self.loadState = initialState.loadState
        self.scanStatus = initialState.scanStatus
        self.ledgerState = initialState.ledgerState
        self.balancePresentation = initialState.balancePresentation
        self.balance = initialState.balance
        self.delegate = delegate
        walletMetadata = initialState.metadata
        unsignedTransactions = initialState.unsignedTransactions

        rustState.withLock { $0.rust = rust }
        rust.listenForUpdates(reconciler: WeakReconciler(self))
    }

    convenience init(id: WalletId, delegate: WalletManagerDelegate? = AppManager.shared) throws {
        let rust = try RustWalletManager(id: id)
        let initialState = rust.initialState()

        self.init(rust: rust, initialState: initialState, delegate: delegate)
    }

    @MainActor
    static func load(id: WalletId, delegate: WalletManagerDelegate? = AppManager.shared) async throws -> WalletManager {
        let bootstrap = try await loadBootstrapAfterPreviousManagerCloses(id: id)

        do {
            try Task.checkCancellation()
        } catch {
            throw error
        }

        return WalletManager(
            rust: bootstrap.rust,
            initialState: bootstrap.initialState,
            delegate: delegate
        )
    }

    private static func loadBootstrapAfterPreviousManagerCloses(
        id: WalletId
    ) async throws -> WalletManagerBootstrap {
        while true {
            do {
                return try await Task.detached(priority: .userInitiated) {
                    let rust = try RustWalletManager(id: id)
                    let initialState = rust.initialState()

                    return WalletManagerBootstrap(rust: rust, initialState: initialState)
                }.value
            } catch {
                guard walletManagerIsClosing(error, walletId: id)
                    || walletManagerConstructionIsInProgress(error, walletId: id)
                else { throw error }
                try await Task.sleep(for: .milliseconds(50))
            }
        }
    }

    func close() {
        guard takeRustForClose() != nil else { return }

        setInitialScanLifecycleChanged(nil)
        logger.debug("Closed WalletManager for wallet \(id)")
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

    private func takeRustForClose() -> RustWalletManager? {
        rustState.withLock { state in
            guard !state.isClosed else { return nil }

            state.isClosed = true
            let rust = state.rust
            state.rust = nil
            return rust
        }
    }

    private func requireRust() throws -> RustWalletManager {
        guard let rust else { throw WalletManagerAccessError.closed }
        return rust
    }

    private func withRust<T>(_ body: (RustWalletManager) throws -> T) throws -> T {
        try body(requireRust())
    }

    private func withRustOr<T>(
        _ defaultValue: T,
        _ body: (RustWalletManager) throws -> T
    ) -> T {
        guard let rust else { return defaultValue }
        return (try? body(rust)) ?? defaultValue
    }

    private func withRustAsync<T>(
        _ body: (RustWalletManager) async throws -> T
    ) async throws -> T {
        let rust = try requireRust()
        return try await body(rust)
    }

    var canApplyReconcileMessages: Bool {
        rust != nil
    }

    convenience init(xpub: String, delegate: WalletManagerDelegate? = AppManager.shared) throws {
        let rust = try RustWalletManager.tryNewFromXpub(xpub: xpub)
        let initialState = rust.initialState()

        self.init(rust: rust, initialState: initialState, delegate: delegate)
    }

    convenience init(
        tapSigner: TapSigner,
        deriveInfo: DeriveInfo,
        backup: Data? = nil,
        birthday: WalletBirthday? = nil,
        delegate: WalletManagerDelegate? = AppManager.shared
    ) throws {
        let rust = try RustWalletManager.tryNewFromTapSigner(
            tapSigner: tapSigner,
            deriveInfo: deriveInfo,
            backup: backup,
            birthday: birthday
        )
        let initialState = rust.initialState()

        self.init(rust: rust, initialState: initialState, delegate: delegate)
    }

    var unit: String {
        switch walletMetadata.selectedUnit {
        case .btc: "btc"
        case .sat: "sats"
        }
    }

    private var amountFormatter: AmountFormatter {
        AmountFormatter(metadata: walletMetadata)
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

    func deleteWallet() async throws {
        try await withRustAsync { try await $0.deleteWallet() }
    }

    func retryDeleteWallet(attemptId: ShutdownAttemptId) async throws {
        try await withRustAsync { try await $0.retryDeleteWallet(attemptId: attemptId) }
    }

    func setWalletType(_ walletType: WalletType) async throws {
        try await withRustAsync { try await $0.setWalletType(walletType: walletType) }
    }

    func validateMetadata() async throws {
        try await withRustAsync { try await $0.validateMetadata() }
    }

    func markWalletAsVerified() async throws {
        try await withRustAsync { try await $0.markWalletAsVerified() }
    }

    func forceWalletScan() async {
        _ = try? await withRustAsync { try await $0.forceWalletScan() }
    }

    @MainActor
    func startWalletScanIfNeeded() async throws {
        guard !walletScanStarted else { return }
        walletScanStarted = true

        do {
            try await withRustAsync { try await $0.startWalletScan() }
        } catch {
            walletScanStarted = false
            throw error
        }
    }

    func firstAddress() async throws -> AddressInfo {
        try await withRustAsync { try await $0.addressAt(index: 0) }
    }

    func newCoinControlManager() async throws -> RustCoinControlManager {
        try await withRustAsync { try await $0.newCoinControlManager() }
    }

    func newSendFlowManager() throws -> RustSendFlowManager {
        try withRust { try $0.newSendFlowManager(balance: balance) }
    }

    func labelManager() throws -> LabelManager {
        try withRust { $0.labelManager() }
    }

    func refreshTransactions() async throws {
        try await withRustAsync { try await $0.getTransactions() }
    }

    func forceUpdateHeight() async throws {
        _ = try await withRustAsync { try await $0.forceUpdateHeight() }
    }

    func exportLabelsForQr(density: QrDensity) async throws -> [String] {
        try await withRustAsync { try await $0.exportLabelsForQr(density: density) }
    }

    func exportLabelsForShare() async throws -> LabelExportResult {
        try await withRustAsync { try await $0.exportLabelsForShare() }
    }

    func exportXpubForQr(density: QrDensity) async throws -> [String] {
        try await withRustAsync { try await $0.exportXpubForQr(density: density) }
    }

    func exportXpubForShare() async throws -> XpubExportResult {
        try await withRustAsync { try await $0.exportXpubForShare() }
    }

    func exportTransactionsCsv() async throws -> TransactionExportResult {
        try await withRustAsync { try await $0.exportTransactionsCsv() }
    }

    func deleteUnsignedTransaction(txId: TxId) throws {
        try withRust { try $0.deleteUnsignedTransaction(txId: txId) }
    }

    func deletionWarningMessage() -> String {
        withRustOr("This action cannot be undone.") { try $0.deletionWarningMessage() }
    }

    func masterFingerprint() -> String? {
        withRustOr(nil) { $0.masterFingerprint() }
    }

    func nonDefaultAccountNumber() -> UInt32? {
        withRustOr(nil) { try $0.nonDefaultAccountNumber() }
    }

    func requiredDeletionConfirmations() -> UInt8 {
        withRustOr(1) { $0.requiredDeletionConfirmations() }
    }

    func exposeXprv() throws -> String {
        try withRust { try $0.exposeXprv() }
    }

    func convertAndDisplayFiat(amount: Amount, prices: PriceResponse) -> String {
        withRustOr("") { $0.convertAndDisplayFiat(amount: amount, prices: prices) }
    }

    func broadcastTransaction(_ transaction: BitcoinTransaction) async throws {
        try await withRustAsync { try await $0.broadcastTransaction(signedTransaction: transaction) }
    }

    func initiatePayment(psbt: Psbt, payjoinEndpoint: String?) async throws {
        try await withRustAsync {
            try await $0.initiatePayment(psbt: psbt, payjoinEndpoint: payjoinEndpoint)
        }
    }

    func finalizePsbt(_ psbt: Psbt) async throws -> BitcoinTransaction {
        try await withRustAsync { try await $0.finalizePsbt(psbt: psbt) }
    }

    func splitTransactionOutputs(
        _ outputs: [AddressAndAmount]
    ) async throws -> SplitOutput {
        try await withRustAsync { try await $0.splitTransactionOutputs(outputs: outputs) }
    }

    func wordValidator() throws -> WordValidator {
        try withRust { try $0.wordValidator() }
    }

    func amountFmt(_ amount: Amount) -> String {
        amountFormatter.amountFmt(amount)
    }

    func displayAmount(_ amount: Amount, showUnit: Bool = true) -> String {
        amountFormatter.displayAmount(amount, showUnit: showUnit)
    }

    func displayAmountPendingFmt(_ amount: Amount) -> String? {
        amountFormatter.displayAmountPendingFmt(amount)
    }

    func displayAmountWithDirection(
        _ amount: Amount,
        direction: TransactionDirection
    ) -> String {
        amountFormatter.displayAmountWithDirection(amount, direction: direction)
    }

    func displaySentAndReceivedAmount(_ sentAndReceived: SentAndReceived) -> String {
        amountFormatter.displaySentAndReceivedAmount(sentAndReceived)
    }

    func displayFiatAmount(_ amount: Double, withSuffix: Bool = true) -> String {
        amountFormatter.displayFiatAmount(amount, withSuffix: withSuffix)
    }

    func displayFiatAmountPendingFmt(
        _ amount: Double,
        withSuffix: Bool = true
    ) -> String? {
        amountFormatter.displayFiatAmountPendingFmt(amount, withSuffix: withSuffix)
    }

    func displayFiatAmountWithDirection(
        _ amount: Double,
        direction: TransactionDirection,
        withSuffix: Bool = true
    ) -> String {
        amountFormatter.displayFiatAmountWithDirection(
            amount,
            direction: direction,
            withSuffix: withSuffix
        )
    }

    func amountInFiatCached(_ amount: Amount) -> Double? {
        amountFormatter.amountInFiatCached(amount)
    }

    func displayConfirmationCount(_ confirmations: UInt32) -> String {
        withRustOr(String(confirmations)) {
            $0.displayConfirmationCount(confirmations: confirmations)
        }
    }

    func amountFmtUnit(_ amount: Amount) -> String {
        amountFormatter.amountFmtUnit(amount)
    }

    func transactionDetails(for txId: TxId) async throws -> TransactionDetailsPresentation {
        if let presentation = await MainActor.run(
            body: { transactionDetailsPresentations[txId] }
        ) {
            return presentation
        }

        let presentation = try await withRustAsync {
            try await $0.transactionDetails(txId: txId)
        }

        await MainActor.run {
            transactionDetailsPresentations[txId] = presentation
        }

        return presentation
    }

    func refreshTransactionDetails(for txId: TxId) async throws -> TransactionDetailsPresentation {
        let presentation = try await withRustAsync {
            try await $0.transactionDetails(txId: txId)
        }

        await MainActor.run {
            transactionDetailsPresentations[txId] = presentation
        }

        return presentation
    }

    func transactionLockState(for txId: TxId) async throws -> TransactionLockState {
        let state = try await withRustAsync {
            try await $0.transactionLockState(txId: txId)
        }

        await MainActor.run {
            transactionLockStates[txId] = state
        }

        return state
    }

    func toggleTransactionLockState(for txId: TxId) async throws -> TransactionLockState {
        let state = try await withRustAsync {
            try await $0.toggleTransactionLockState(txId: txId)
        }

        await MainActor.run {
            transactionLockStates[txId] = state
        }
        await delegate?.reconcileAfterLabelsChanged(walletId: id)

        return state
    }

    func unlockTransactionOutputs(for txId: TxId) async throws -> TransactionLockState {
        let state = try await withRustAsync {
            try await $0.unlockTransactionOutputs(txId: txId)
        }

        await MainActor.run {
            transactionLockStates[txId] = state
        }
        await delegate?.reconcileAfterLabelsChanged(walletId: id)

        return state
    }

    @MainActor
    func clearTransactionLockState(for txId: TxId) {
        transactionLockStates[txId] = nil
    }

    @MainActor
    func importLabels(labels: Bip329Labels) throws {
        try withRust { try $0.labelManager().import(labels: labels) }
        delegate?.reconcileAfterLabelsChanged(walletId: id)
    }

    @MainActor
    func reconcileAfterLabelsChanged() {
        let cachedTransactionIds = Array(transactionDetailsPresentations.keys)
        let cachedLockStateTransactionIds = Array(transactionLockStates.keys)

        Task {
            for txId in cachedTransactionIds {
                do {
                    _ = try await refreshTransactionDetails(for: txId)
                } catch {
                    logger.error("Failed to refresh transaction details after label change: \(error)")
                }
            }

            for txId in cachedLockStateTransactionIds {
                do {
                    _ = try await transactionLockState(for: txId)
                } catch {
                    logger.error("Failed to refresh transaction lock state after label change: \(error)")
                    await clearTransactionLockState(for: txId)
                }
            }

            _ = try? await withRustAsync { try await $0.getTransactions() }
        }
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
        guard let balance = try? await withRustAsync({ try await $0.balance() }) else { return }

        await MainActor.run {
            self.balance = balance
        }
    }

    func apply(_ message: Message) {
        switch message {
        case .walletScanStatusChanged, .ledgerStateChanged, .scanComplete:
            applyScanLifecycleMessage(message)
        case .availableTransactions, .updatedTransactions, .transactionUpdated,
             .transactionDetailsUpdated:
            applyTransactionMessage(message)
        case .walletBalanceChanged, .unsignedTransactionsChanged, .walletMetadataChanged,
             .walletScannerResponse, .nodeConnectionFailed, .walletError, .unknownError,
             .sendFlowError, .hotWalletKeyMissing, .payjoinTxBroadcast,
             .payjoinPollingStarted:
            applyWalletStateMessage(message)
        case .receiveAddressUpdated, .receiveAddressPresentationUpdated,
             .receiveAddressLoadingChanged, .receiveAddressError, .receiveAddressClosed:
            applyReceiveAddressMessage(message)
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

    private func recomputeLedgerStateForMetadataChange() {
        ledgerState = if walletMetadata.internal.performedFullScanAt == nil {
            .initialScanIncomplete(scanStatus.isActive ? .active : .idle)
        } else {
            .complete
        }
        balancePresentation = withRustOr(balancePresentation) {
            $0.balancePresentationForState(ledgerState: ledgerState)
        }
        reconcileLoadStateWithLedgerState()
        notifyInitialScanLifecycleChanged()
    }

    func logReconcile(message: Message) {
        logger.debug("reconcile \(message)")
    }

    func logReconcileMany(messages: [Message]) {
        logger.debug("reconcile_messages: \(messages)")
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
            guard let self, let rust = self.rust else { return }

            self.logger.debug("dispatch: \(action)")
            try? rust.dispatch(action: action)
        }
    }

    /// PREVIEW only
    convenience init(preview _: WalletManagerPreview, _ walletMetadata: WalletMetadata? = nil) {
        let rust =
            if let walletMetadata {
                RustWalletManager.previewNewWalletWithMetadata(metadata: walletMetadata)
            } else {
                RustWalletManager.previewNewWallet()
            }

        let initialState = rust.initialState()

        self.init(rust: rust, initialState: initialState, delegate: nil)
    }

    deinit {
        close()
        logger.debug("WalletManager deinit called for wallet \(id)")
    }
}

extension WalletManager {
    private func applyScanLifecycleMessage(_ message: Message) {
        switch message {
        case let .walletScanStatusChanged(status):
            scanStatus = status
            balancePresentation = withRustOr(balancePresentation) {
                $0.balancePresentationForState(ledgerState: ledgerState)
            }
            if status.isActive {
                switch loadState {
                case .scanning:
                    break
                case let .loaded(transactions):
                    loadState = .scanning(transactions)
                case .loading:
                    loadState = .scanning([])
                }
            } else if case let .scanning(transactions) = loadState,
                      ledgerState.initialScanComplete
            {
                loadState = .loaded(transactions)
            }
            notifyInitialScanLifecycleChanged()

        case let .ledgerStateChanged(ledgerState):
            self.ledgerState = ledgerState
            balancePresentation = withRustOr(balancePresentation) {
                $0.balancePresentationForState(ledgerState: ledgerState)
            }
            reconcileLoadStateWithLedgerState()
            notifyInitialScanLifecycleChanged()

        case let .scanComplete(transactions):
            loadState = loadStateForTransactions(transactions)
            notifyInitialScanLifecycleChanged()

        default:
            preconditionFailure("Expected a wallet scan lifecycle reconcile message")
        }
    }

    private func applyTransactionMessage(_ message: Message) {
        switch message {
        // a cache replay proves nothing about the network, so it must not clear `errorAlert`
        case let .availableTransactions(transactions):
            switch loadState {
            case .loading:
                loadState = loadStateForTransactions(transactions)
            case let .scanning(current) where transactions.count >= current.count:
                loadState = loadStateForTransactions(transactions)
            case .scanning:
                break
            case let .loaded(current) where transactions.count >= current.count:
                loadState = loadStateForTransactions(transactions)
            case .loaded:
                break
            }

        case let .updatedTransactions(transactions):
            loadState = loadStateForTransactions(transactions)

        case let .transactionUpdated(transaction):
            replaceTransactionInLoadState(transaction)

        case let .transactionDetailsUpdated(presentation):
            transactionDetailsPresentations[presentation.txId()] = presentation

        default:
            preconditionFailure("Expected a wallet transaction reconcile message")
        }
    }

    private func applyWalletStateMessage(_ message: Message) {
        switch message {
        case let .walletBalanceChanged(balance):
            withAnimation { self.balance = balance }

        case .unsignedTransactionsChanged:
            do {
                unsignedTransactions = try withRust { try $0.getUnsignedTransactions() }
            } catch {
                logger.error(
                    "Unable to refresh unsigned transactions: \(error.localizedDescription)"
                )
                unsignedTransactions = []
            }

        case let .walletMetadataChanged(metadata):
            withAnimation { walletMetadata = metadata }
            recomputeLedgerStateForMetadataChange()

        case let .walletScannerResponse(scannerResponse):
            logger.debug("walletScannerResponse: \(scannerResponse)")
            if case let .foundAddresses(addressTypes) = scannerResponse {
                foundAddresses = addressTypes
            }

        case let .nodeConnectionFailed(error):
            errorAlert = TaggedItem(WalletErrorAlert.nodeConnectionFailed(error))
            logger.error(error)
            logger.error("set errorAlert")

        case let .walletError(error):
            logger.error("WalletError \(error)")
            payjoinDeadlineSecs = nil

        case let .unknownError(error):
            // TODO: show to user
            logger.error("Unknown error \(error)")

        case let .sendFlowError(error):
            sendFlowErrorAlert = TaggedItem(error)
            payjoinDeadlineSecs = nil

        case let .hotWalletKeyMissing(walletId):
            delegate?.showWalletAlert(.hotWalletKeyMissing(walletId: walletId))

        case .payjoinTxBroadcast:
            payjoinTxBroadcast = UUID()
            payjoinDeadlineSecs = nil

        case let .payjoinPollingStarted(deadlineSecs):
            payjoinDeadlineSecs = deadlineSecs

        default:
            preconditionFailure("Expected a wallet state reconcile message")
        }
    }

    private func applyReceiveAddressMessage(_ message: Message) {
        switch message {
        case let .receiveAddressUpdated(state):
            receiveAddressState = state

        case let .receiveAddressPresentationUpdated(presentation):
            receiveAddressPresentation = presentation

        case let .receiveAddressLoadingChanged(isLoading):
            receiveAddressIsLoading = isLoading

        case let .receiveAddressError(error):
            receiveAddressError = TaggedString(error)

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

        default:
            preconditionFailure("Expected a receive address reconcile message")
        }
    }
}

extension WalletLoadState: @retroactive Equatable {
    public static func == (lhs: WalletLoadState, rhs: WalletLoadState) -> Bool {
        lhs.isEqual(other: rhs)
    }
}
