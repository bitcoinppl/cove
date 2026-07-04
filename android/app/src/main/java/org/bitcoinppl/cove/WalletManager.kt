package org.bitcoinppl.cove

import androidx.compose.runtime.Stable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateMapOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.runtime.snapshots.SnapshotStateMap
import androidx.compose.ui.graphics.Color
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import org.bitcoinppl.cove.ui.theme.CoveColor
import org.bitcoinppl.cove.utils.toComposeColor
import org.bitcoinppl.cove_core.*
import org.bitcoinppl.cove_core.tapcard.TapSigner
import org.bitcoinppl.cove_core.types.*
import java.io.Closeable
import java.util.concurrent.atomic.AtomicBoolean

private val WalletScanStatus.isActive: Boolean
    get() =
        when (this) {
            WalletScanStatus.Idle -> false
            is WalletScanStatus.Scanning, is WalletScanStatus.ScanningPendingProgress -> true
        }

val WalletLedgerState.initialScanComplete: Boolean
    get() = this is WalletLedgerState.Complete

val WalletLedgerState.initialScanIncomplete: Boolean
    get() = !initialScanComplete

val WalletLedgerState.initialScanActive: Boolean
    get() = this is WalletLedgerState.InitialScanIncomplete && v1 == InitialScanActivity.ACTIVE

/**
 * wallet manager - manages wallet state, balance, transactions
 * ported from iOS WalletManager.swift
 */
@Stable
class WalletManager :
    WalletManagerReconciler,
    Closeable {
    private val tag = "WalletManager"

    private val mainScope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)
    private val ioScope = CoroutineScope(SupervisorJob() + Dispatchers.IO)
    private val isClosed = AtomicBoolean(false)

    val id: WalletId
    private val rust: RustWalletManager
    private val walletScanStarted = AtomicBoolean(false)

    // observable state
    var walletMetadata by mutableStateOf<WalletMetadata?>(null)
        private set

    var ledgerState by mutableStateOf<WalletLedgerState>(WalletLedgerState.InitialScanIncomplete(InitialScanActivity.IDLE))
        private set

    var loadState by mutableStateOf<WalletLoadState>(WalletLoadState.Loading)
        private set

    var scanStatus by mutableStateOf<WalletScanStatus>(WalletScanStatus.Idle)
        private set

    private var balancePresentationState by mutableStateOf(
        BalancePresentation(
            primaryOpacity = 1.0,
            secondaryOpacity = 0.75,
            pendingOpacity = 0.6,
        ),
    )

    val balancePresentation: BalancePresentation
        get() = balancePresentationState

    var balance by mutableStateOf(Balance.zero())
        private set

    var foundAddresses by mutableStateOf<List<FoundAddress>>(emptyList())
        private set

    var unsignedTransactions by mutableStateOf<List<UnsignedTransaction>>(emptyList())
        private set

    // errors
    var errorAlert by mutableStateOf<WalletErrorAlert?>(null)
    var sendFlowErrorAlert by mutableStateOf<TaggedItem<SendFlowErrorAlert>?>(null)
    var labelRefreshFailed by mutableStateOf<TaggedItem<Unit>?>(null)
        private set

    // non-null when a payjoin transaction has been broadcast (success or fallback);
    // TaggedItem ensures a new unique key each time so Compose always re-fires the observer
    var payjoinTxBroadcast by mutableStateOf<TaggedItem<Unit>?>(null)

    // cached transaction details (observable for Compose)
    val transactionDetailsCache: SnapshotStateMap<TxId, TransactionDetails> = mutableStateMapOf()
    val transactionConfirmations: SnapshotStateMap<TxId, UInt> = mutableStateMapOf()
    val transactionLockStates: SnapshotStateMap<TxId, TransactionLockState> = mutableStateMapOf()

    var receiveAddressState by mutableStateOf<ReceiveAddressState?>(null)
    var receiveAddressPresentation by mutableStateOf(
        ReceiveAddressPresentation(
            copyPolicy = ReceiveAddressCopyPolicy.COPY,
            refreshState = ReceiveAddressRefreshState.IDLE,
        ),
    )
    var receiveAddressIsLoading by mutableStateOf(false)
    var receiveAddressError by mutableStateOf<TaggedItem<String>?>(null)

    // scroll position for transaction list (persists across navigation)
    var scrolledTransactionId: String? by mutableStateOf(null)

    // pending scroll ID set when clicking a transaction, transferred to scrolledTransactionId
    // when returning to the wallet screen (avoids scrolling during navigation transition)
    var pendingScrollTransactionId: String? = null

    // computed properties
    val unit: String
        get() =
            when (walletMetadata?.selectedUnit) {
                BitcoinUnit.BTC -> "btc"
                BitcoinUnit.SAT -> "sats"
                else -> "sats"
            }

    val hasTransactions: Boolean
        get() =
            when (val state = loadState) {
                is WalletLoadState.Loading -> false
                is WalletLoadState.Scanning -> state.v1.isNotEmpty()
                is WalletLoadState.Loaded -> state.v1.isNotEmpty()
            }

    val isVerified: Boolean
        get() = walletMetadata?.verified ?: false

    val accentColor: Color
        get() = walletMetadata?.color?.toComposeColor() ?: CoveColor.pastelBlue

    private val requiredWalletMetadata: WalletMetadata
        get() = walletMetadata ?: error("wallet metadata is not initialized")

    // private constructor - use companion factory methods
    private constructor(
        walletId: WalletId,
        rustManager: RustWalletManager,
        initialState: WalletInitialState,
    ) {
        this.id = walletId
        this.rust = rustManager
        this.walletMetadata = initialState.metadata
        this.ledgerState = initialState.ledgerState
        this.loadState = initialState.loadState
        this.scanStatus = initialState.scanStatus
        this.balancePresentationState = initialState.balancePresentation
        this.balance = initialState.balance
        this.unsignedTransactions = initialState.unsignedTransactions

        rustManager.listenForUpdates(this)
    }

    companion object {
        // create from wallet ID
        operator fun invoke(id: WalletId): WalletManager {
            val rust = RustWalletManager(id)
            val initialState = rust.initialState()
            android.util.Log.d("WalletManager", "Initialized WalletManager for $id")
            return WalletManager(initialState.metadata.id, rust, initialState)
        }

        // create from xpub
        fun fromXpub(xpub: String): WalletManager {
            val rust = RustWalletManager.tryNewFromXpub(xpub)
            val initialState = rust.initialState()
            android.util.Log.d("WalletManager", "Initialized WalletManager from xpub")
            return WalletManager(initialState.metadata.id, rust, initialState)
        }

        // create from TapSigner
        fun fromTapSigner(
            tapSigner: TapSigner,
            deriveInfo: DeriveInfo,
            backup: ByteArray? = null,
            birthday: WalletBirthday? = null,
        ): WalletManager {
            val rust =
                RustWalletManager.tryNewFromTapSigner(
                    tapSigner,
                    deriveInfo,
                    backup,
                    birthday,
                )
            val initialState = rust.initialState()
            android.util.Log.d("WalletManager", "Initialized WalletManager from TapSigner")
            return WalletManager(initialState.metadata.id, rust, initialState)
        }

        internal fun previewNew(): WalletManager {
            val rust = RustWalletManager.previewNewWallet()
            val initialState = rust.initialState()
            return WalletManager(initialState.metadata.id, rust, initialState)
        }
    }

    private fun logDebug(message: String) {
        android.util.Log.d(tag, message)
    }

    private fun logError(message: String, throwable: Throwable? = null) {
        if (throwable != null) {
            android.util.Log.e(tag, message, throwable)
        } else {
            android.util.Log.e(tag, message)
        }
    }

    private val rustGuard =
        RustHandleGuard(
            ownerName = "WalletManager",
            handleName = "RustWalletManager",
            isClosed = isClosed,
        ) {
            android.util.Log.w(tag, it)
        }

    private fun <T> withRust(
        block: RustWalletManager.() -> T,
    ): T = rustGuard.withHandle(rust, block)

    private fun <T> withRustOr(
        defaultValue: T,
        block: RustWalletManager.() -> T,
    ): T = rustGuard.withHandleOr(rust, defaultValue, block)

    private suspend fun <T> withRustSuspend(
        block: suspend RustWalletManager.() -> T,
    ): T = rustGuard.withHandleSuspend(rust, block)

    private suspend fun <T> withRustOrSuspend(
        defaultValue: T,
        block: suspend RustWalletManager.() -> T,
    ): T = rustGuard.withHandleOrSuspend(rust, defaultValue, block)

    fun validateMetadata() {
        withRustOr(Unit) {
            validateMetadata()
        }
    }

    suspend fun forceWalletScan() {
        withRustSuspend {
            forceWalletScan()
        }
    }

    suspend fun startWalletScan() {
        withRustSuspend {
            startWalletScan()
        }
    }

    suspend fun startWalletScanIfNeeded() {
        if (!walletScanStarted.compareAndSet(false, true)) return

        try {
            startWalletScan()
        } catch (e: Exception) {
            walletScanStarted.set(false)
            throw e
        }
    }

    fun setScanning() {
        val currentTxns =
            when (val state = loadState) {
                is WalletLoadState.Loaded -> state.v1
                is WalletLoadState.Scanning -> state.v1
                else -> emptyList()
            }
        loadState = WalletLoadState.Scanning(currentTxns)
    }

    suspend fun firstAddress(): AddressInfo =
        withRustSuspend {
            addressAt(0u)
        }

    internal fun newSendFlowManager(balance: Balance): RustSendFlowManager =
        withRust {
            newSendFlowManager(balance)
        }

    suspend fun newCoinControlManager(): CoinControlManager =
        CoinControlManager(
            withRustSuspend {
                newCoinControlManager()
            },
        )

    suspend fun refreshTransactions() {
        withRustSuspend {
            getTransactions()
        }
    }

    suspend fun forceUpdateHeight(): UInt =
        withRustSuspend {
            forceUpdateHeight()
        }

    fun labelManager(): LabelManager =
        withRust {
            labelManager()
        }

    fun hasLabels(): Boolean =
        withRustOr(false) {
            labelManager().use { it.hasLabels() }
        }

    suspend fun switchToDifferentWalletAddressType(type: WalletAddressType) {
        withRustSuspend {
            switchToDifferentWalletAddressType(type)
        }
    }

    fun deleteWallet() {
        withRust {
            deleteWallet()
        }
    }

    fun deletionWarningMessage(): String =
        withRustOr("") {
            deletionWarningMessage()
        }

    fun requiredDeletionConfirmations(): UByte =
        withRustOr(1u) {
            requiredDeletionConfirmations()
        }

    fun nonDefaultAccountNumber(): UInt? =
        withRustOr(null) {
            nonDefaultAccountNumber()
        }

    fun masterFingerprint(): String? =
        withRustOr(null) {
            masterFingerprint()
        }

    suspend fun exportLabelsForQr(density: QrDensity): List<String> =
        withRustOrSuspend(emptyList()) {
            exportLabelsForQr(density)
        }

    suspend fun exportLabelsForShare(): LabelExportResult =
        withRustSuspend {
            exportLabelsForShare()
        }

    suspend fun exportTransactionsCsv(): TransactionExportResult =
        withRustSuspend {
            exportTransactionsCsv()
        }

    suspend fun exportXpubForQr(density: QrDensity): List<String> =
        withRustOrSuspend(emptyList()) {
            exportXpubForQr(density)
        }

    suspend fun exportXpubForShare(): XpubExportResult =
        withRustSuspend {
            exportXpubForShare()
        }

    fun setWalletType(walletType: WalletType) {
        withRust {
            setWalletType(walletType)
        }
    }

    fun markWalletAsVerified() {
        withRust {
            markWalletAsVerified()
        }
    }

    fun wordValidator(): WordValidator =
        withRust {
            wordValidator()
        }

    fun deleteUnsignedTransaction(txnId: TxId) {
        withRust {
            deleteUnsignedTransaction(txnId)
        }
    }

    suspend fun deleteUnsignedTransactionAsync(txnId: TxId) {
        withRustSuspend {
            deleteUnsignedTransaction(txnId)
        }
    }

    suspend fun splitTransactionOutputs(outputs: List<AddressAndAmount>): SplitOutput =
        withRustSuspend {
            splitTransactionOutputs(outputs)
        }

    fun convertAndDisplayFiat(amount: Amount, prices: PriceResponse): String =
        withRustOr("") {
            convertAndDisplayFiat(amount, prices)
        }

    suspend fun finalizePsbt(psbt: Psbt): BitcoinTransaction =
        withRustSuspend {
            finalizePsbt(psbt)
        }

    suspend fun broadcastTransaction(transaction: BitcoinTransaction) {
        withRustSuspend {
            broadcastTransaction(transaction)
        }
    }

    suspend fun initiatePayment(psbt: Psbt, payjoinEndpoint: String?) {
        withRustSuspend {
            initiatePayment(psbt, payjoinEndpoint)
        }
    }

    suspend fun numberOfConfirmations(blockHeight: UInt): UInt =
        withRustSuspend {
            numberOfConfirmations(blockHeight)
        }

    fun displayConfirmationCount(confirmations: UInt): String =
        withRustOr("") {
            displayConfirmationCount(confirmations)
        }

    fun amountFmt(amount: Amount): String =
        when (walletMetadata?.selectedUnit) {
            BitcoinUnit.BTC -> amount.btcString()
            BitcoinUnit.SAT -> amount.satsString()
            else -> amount.satsString()
        }

    fun displayAmount(amount: Amount, showUnit: Boolean = true): String {
        return walletDisplayAmount(requiredWalletMetadata, amount, showUnit)
    }

    fun displayAmountPendingFmt(amount: Amount): String? {
        return walletDisplayAmountPendingFmt(requiredWalletMetadata, amount)
    }

    fun displayAmountWithDirection(
        amount: Amount,
        direction: TransactionDirection,
    ): String {
        return walletDisplayAmountWithDirection(requiredWalletMetadata, amount, direction)
    }

    fun displaySentAndReceivedAmount(sentAndReceived: SentAndReceived): String {
        return walletDisplaySentAndReceivedAmount(requiredWalletMetadata, sentAndReceived)
    }

    fun displayFiatAmount(
        amount: Double,
        withSuffix: Boolean = true,
    ): String {
        return walletDisplayFiatAmount(requiredWalletMetadata, amount, withSuffix)
    }

    fun displayFiatAmountPendingFmt(
        amount: Double,
        withSuffix: Boolean = true,
    ): String? {
        return walletDisplayFiatAmountPendingFmt(requiredWalletMetadata, amount, withSuffix)
    }

    fun displayFiatAmountWithDirection(
        amount: Double,
        direction: TransactionDirection,
        withSuffix: Boolean = true,
    ): String {
        return walletDisplayFiatAmountWithDirection(requiredWalletMetadata, amount, direction, withSuffix)
    }

    fun amountInFiatCached(amount: Amount): Double? = walletAmountInFiatCached(amount)

    fun amountFmtUnit(amount: Amount): String =
        when (walletMetadata?.selectedUnit) {
            BitcoinUnit.BTC -> amount.btcStringWithUnit()
            BitcoinUnit.SAT -> amount.satsStringWithUnit()
            else -> amount.satsStringWithUnit()
        }

    suspend fun transactionDetails(txId: TxId): TransactionDetails {
        // check cache first
        transactionDetailsCache[txId]?.let { return it }

        // fetch from rust and cache
        val details =
            withRustSuspend {
                transactionDetails(txId)
            }
        transactionDetailsCache[txId] = details
        return details
    }

    suspend fun refreshTransactionDetails(txId: TxId): TransactionDetails {
        val details =
            withRustSuspend {
                transactionDetails(txId)
            }
        transactionDetailsCache[txId] = details

        val blockNumber = details.blockNumber()
        if (blockNumber != null) {
            transactionConfirmations[txId] =
                withRustSuspend {
                    numberOfConfirmations(blockNumber)
                }
        }

        return details
    }

    suspend fun transactionLockState(txId: TxId): TransactionLockState {
        val state =
            withRustSuspend {
                transactionLockState(txId)
            }
        transactionLockStates[txId] = state

        return state
    }

    suspend fun toggleTransactionLockState(txId: TxId): TransactionLockState {
        val state =
            withRustSuspend {
                toggleTransactionLockState(txId)
            }
        transactionLockStates[txId] = state
        AppManager.getInstance().reconcileAfterLabelImport(id)

        return state
    }

    suspend fun unlockTransactionOutputs(txId: TxId): TransactionLockState {
        val state =
            withRustSuspend {
                unlockTransactionOutputs(txId)
            }
        transactionLockStates[txId] = state
        AppManager.getInstance().reconcileAfterLabelImport(id)

        return state
    }

    fun clearTransactionLockState(txId: TxId) {
        transactionLockStates.remove(txId)
    }

    fun importLabels(labels: Bip329Labels) {
        LabelManager(id = id).use { it.importLabels(labels) }
        AppManager.getInstance().reconcileAfterLabelImport(id)
    }

    suspend fun reconcileAfterLabelImportAndWait(): Boolean {
        val cachedTransactionIds = transactionDetailsCache.keys.toList()
        val cachedLockStateTransactionIds = transactionLockStates.keys.toList()
        var refreshedDetails = true

        for (txId in cachedTransactionIds) {
            val refreshed =
                runCatchingCancellable(tag, "failed to refresh transaction details after label import") {
                    refreshTransactionDetails(txId)
                }.isSuccess
            if (!refreshed) {
                refreshedDetails = false
            }
        }

        for (txId in cachedLockStateTransactionIds) {
            val refreshed =
                runCatchingCancellable(tag, "failed to refresh transaction lock state after label import") {
                    transactionLockState(txId)
                }.isSuccess
            if (!refreshed) {
                clearTransactionLockState(txId)
            }
        }

        return runCatchingCancellable(tag, "failed to refresh transactions after label import") {
            withRustSuspend {
                getTransactions()
            }
            refreshedDetails
        }.getOrDefault(false)
    }

    fun notifyLabelRefreshFailed() {
        labelRefreshFailed = TaggedItem(Unit)
    }

    fun clearLabelRefreshFailed() {
        labelRefreshFailed = null
    }

    fun updateTransactionDetailsCache(txId: TxId, details: TransactionDetails) {
        transactionDetailsCache[txId] = details
    }

    fun updateTransactionConfirmations(
        txId: TxId,
        confirmations: UInt,
    ) {
        transactionConfirmations[txId] = confirmations
    }

    private fun replaceTransactionInLoadState(transaction: Transaction) {
        fun replace(txns: List<Transaction>): List<Transaction> {
            val txId = transaction.txId()
            var replaced = false
            val updated =
                txns.map { current ->
                    if (current.txId() == txId) {
                        replaced = true
                        transaction
                    } else {
                        current
                    }
                }

            return if (replaced) updated else listOf(transaction) + updated
        }

        loadState =
            when (val current = loadState) {
                is WalletLoadState.Loading ->
                    if (ledgerState.initialScanComplete) {
                        WalletLoadState.Loaded(listOf(transaction))
                    } else {
                        WalletLoadState.Scanning(listOf(transaction))
                    }
                is WalletLoadState.Scanning -> WalletLoadState.Scanning(replace(current.v1))
                is WalletLoadState.Loaded -> WalletLoadState.Loaded(replace(current.v1))
            }
    }

    suspend fun updateWalletBalance() {
        val bal =
            withRustSuspend {
                balance()
            }
        withContext(Dispatchers.Main) {
            balance = bal
        }
    }

    private fun apply(message: WalletManagerReconcileMessage) {
        when (message) {
            is WalletManagerReconcileMessage.WalletScanStatusChanged -> {
                scanStatus = message.v1
                balancePresentationState =
                    withRustOr(balancePresentationState) {
                        balancePresentationForState(ledgerState)
                    }
                if (message.v1.isActive) {
                    when (val current = loadState) {
                        is WalletLoadState.Scanning -> Unit
                        is WalletLoadState.Loaded -> loadState = WalletLoadState.Scanning(current.v1)
                        is WalletLoadState.Loading -> loadState = WalletLoadState.Scanning(listOf())
                    }
                } else {
                    when (val current = loadState) {
                        is WalletLoadState.Scanning -> {
                            if (ledgerState.initialScanComplete) {
                                loadState = WalletLoadState.Loaded(current.v1)
                            }
                        }
                        is WalletLoadState.Loaded, is WalletLoadState.Loading -> Unit
                    }
                }
            }

            is WalletManagerReconcileMessage.LedgerStateChanged -> {
                ledgerState = message.v1
                balancePresentationState =
                    withRustOr(balancePresentationState) {
                        balancePresentationForState(message.v1)
                    }
                reconcileLoadStateWithLedgerState()
            }

            is WalletManagerReconcileMessage.AvailableTransactions -> {
                val txns = message.v1
                when (val current = loadState) {
                    is WalletLoadState.Loading -> {
                        loadState = loadStateForTransactions(txns)
                    }
                    is WalletLoadState.Scanning -> {
                        if (txns.size >= current.v1.size) {
                            loadState = loadStateForTransactions(txns)
                        }
                    }
                    is WalletLoadState.Loaded -> {
                        if (txns.size >= current.v1.size) {
                            loadState = loadStateForTransactions(txns)
                        }
                    }
                }
            }

            is WalletManagerReconcileMessage.UpdatedTransactions -> {
                loadState = loadStateForTransactions(message.v1)
            }

            is WalletManagerReconcileMessage.TransactionUpdated -> {
                replaceTransactionInLoadState(message.v1)
            }

            is WalletManagerReconcileMessage.TransactionDetailsUpdated -> {
                transactionDetailsCache[message.v1.txId()] = message.v1
            }

            is WalletManagerReconcileMessage.TransactionConfirmationsUpdated -> {
                transactionConfirmations[message.v1.txId] = message.v1.confirmations
            }

            is WalletManagerReconcileMessage.ScanComplete -> {
                loadState = loadStateForTransactions(message.v1)
            }

            is WalletManagerReconcileMessage.WalletBalanceChanged -> {
                balance = message.v1
            }

            is WalletManagerReconcileMessage.UnsignedTransactionsChanged -> {
                unsignedTransactions =
                    runCatching {
                        withRustOr(emptyList()) {
                            getUnsignedTransactions()
                        }
                    }.getOrElse { error ->
                        logError("Unable to refresh unsigned transactions", error)
                        emptyList()
                    }
            }

            is WalletManagerReconcileMessage.WalletMetadataChanged -> {
                walletMetadata = message.v1
                persistWalletMetadata(message.v1)
            }

            is WalletManagerReconcileMessage.WalletScannerResponse -> {
                logDebug("walletScannerResponse: ${message.v1}")
                when (val response = message.v1) {
                    is ScannerResponse.FoundAddresses -> {
                        foundAddresses = response.v1
                    }
                    else -> {
                        // handle other scanner response types
                    }
                }
            }

            is WalletManagerReconcileMessage.NodeConnectionFailed -> {
                errorAlert = WalletErrorAlert.NodeConnectionFailed(message.v1)
                logError(message.v1)
            }

            is WalletManagerReconcileMessage.WalletException -> {
                logError("WalletException: ${message.v1}")
            }

            is WalletManagerReconcileMessage.UnknownError -> {
                logError("Unknown error: ${message.v1}")
            }

            is WalletManagerReconcileMessage.SendFlowException -> {
                sendFlowErrorAlert = TaggedItem(message.v1)
            }

            is WalletManagerReconcileMessage.HotWalletKeyMissing -> {
                AppManager.getInstance().alertState = TaggedItem(AppAlertState.HotWalletKeyMissing(message.v1))
            }

            is WalletManagerReconcileMessage.ReceiveAddressUpdated -> {
                receiveAddressState = message.v1
            }

            is WalletManagerReconcileMessage.ReceiveAddressPresentationUpdated -> {
                receiveAddressPresentation = message.v1
            }

            is WalletManagerReconcileMessage.ReceiveAddressLoadingChanged -> {
                receiveAddressIsLoading = message.v1
            }

            is WalletManagerReconcileMessage.ReceiveAddressError -> {
                receiveAddressError = TaggedItem(message.v1)
            }

            is WalletManagerReconcileMessage.ReceiveAddressClosed -> {
                if (receiveAddressState?.requestId == message.v1) {
                    receiveAddressState = null
                    receiveAddressPresentation =
                        ReceiveAddressPresentation(
                            copyPolicy = ReceiveAddressCopyPolicy.COPY,
                            refreshState = ReceiveAddressRefreshState.IDLE,
                        )
                    receiveAddressIsLoading = false
                    receiveAddressError = null
                }
            }

            is WalletManagerReconcileMessage.PayjoinTxBroadcast -> {
                payjoinTxBroadcast = TaggedItem(Unit)
            }
        }
    }

    override fun reconcile(message: WalletManagerReconcileMessage) {
        mainScope.launch {
            logDebug("reconcile: $message")
            apply(message)
        }
    }

    override fun reconcileMany(messages: List<WalletManagerReconcileMessage>) {
        mainScope.launch {
            logDebug("reconcile_messages: ${messages.size} messages")
            messages.forEach { apply(it) }
        }
    }

    fun dispatch(action: WalletManagerAction) {
        when (action) {
            is WalletManagerAction.OpenReceiveAddress,
            is WalletManagerAction.CreateNewReceiveAddress -> receiveAddressError = null
            else -> Unit
        }

        logDebug("dispatch: $action")
        mainScope.launch(Dispatchers.IO) {
            withRustOr(Unit) {
                dispatch(action)
            }
        }
    }

    private fun Transaction.txId(): TxId =
        when (this) {
            is Transaction.Confirmed -> v1.id()
            is Transaction.Unconfirmed -> v1.id()
        }

    private fun persistWalletMetadata(metadata: WalletMetadata) {
        ioScope.launch {
            withRustOr(Unit) {
                setWalletMetadata(metadata)
            }
        }
    }

    private fun reconcileLoadStateWithLedgerState() {
        when (val current = loadState) {
            is WalletLoadState.Scanning -> {
                loadState = loadStateForTransactions(current.v1)
            }
            is WalletLoadState.Loaded -> {
                loadState = loadStateForTransactions(current.v1)
            }
            is WalletLoadState.Loading -> Unit
        }
    }

    private fun loadStateForTransactions(transactions: List<Transaction>): WalletLoadState =
        when {
            scanStatus.isActive -> WalletLoadState.Scanning(transactions)
            ledgerState.initialScanComplete -> WalletLoadState.Loaded(transactions)
            transactions.isEmpty() -> WalletLoadState.Loading
            else -> WalletLoadState.Scanning(transactions)
        }

    override fun close() {
        rustGuard.closeOnce {
            logDebug("Closing WalletManager for $id")
            rust.shutdown()
            ioScope.cancel()
            mainScope.cancel()
            rust.close()
        }
    }
}
