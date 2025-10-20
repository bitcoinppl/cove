package org.bitcoinppl.cove

import androidx.compose.runtime.Stable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.graphics.Color
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

/**
 * wallet manager - manages wallet state, balance, transactions
 * ported from iOS WalletManager.swift
 */
@Stable
class WalletManager : WalletManagerReconciler {
    private val tag = "WalletManager"

    private val mainScope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)

    val id: WalletId
    internal val rust: RustWalletManager

    // observable state
    var walletMetadata by mutableStateOf<WalletMetadata?>(null)
        private set

    var loadState by mutableStateOf<WalletLoadState>(WalletLoadState.LOADING)
        private set

    var balance by mutableStateOf(Balance.zero())
        private set

    var fiatBalance by mutableStateOf<Double?>(null)
        private set

    var foundAddresses by mutableStateOf<List<FoundAddress>>(emptyList())
        private set

    var unsignedTransactions by mutableStateOf<List<UnsignedTransaction>>(emptyList())
        private set

    // errors
    var errorAlert by mutableStateOf<WalletErrorAlert?>(null)
    var sendFlowErrorAlert by mutableStateOf<TaggedItem<SendFlowErrorAlert>?>(null)

    // cached transaction details
    private val transactionDetailsCache = mutableMapOf<TxId, TransactionDetails>()

    // computed properties
    val unit: String
        get() =
            when (walletMetadata?.selectedUnit) {
                Unit.BTC -> "btc"
                Unit.SAT -> "sats"
                else -> "sats"
            }

    val hasTransactions: Boolean
        get() =
            when (loadState) {
                is WalletLoadState.LOADING -> false
                is WalletLoadState.SCANNING -> (loadState as WalletLoadState.SCANNING).txns.isNotEmpty()
                is WalletLoadState.LOADED -> (loadState as WalletLoadState.LOADED).txns.isNotEmpty()
            }

    val isVerified: Boolean
        get() = walletMetadata?.verified ?: false

    val accentColor: Color
        get() =
            walletMetadata?.let { metadata ->
                // convert WalletColor to Compose Color
                // TODO: implement proper color conversion from metadata.color
                Color.Blue
            } ?: Color.Blue

    // primary constructor
    constructor(id: WalletId) {
        this.id = id
        val rust = RustWalletManager(id)
        this.rust = rust

        walletMetadata = rust.walletMetadata()
        unsignedTransactions = runCatching { rust.getUnsignedTransactions() }.getOrElse { emptyList() }

        // start fiat balance update
        mainScope.launch(Dispatchers.IO) { updateFiatBalance() }

        rust.listenForUpdates(this)
        logDebug("Initialized WalletManager for $id")
    }

    // constructor from xpub
    constructor(xpub: String) {
        val rust = RustWalletManager.tryNewFromXpub(xpub)
        val metadata = rust.walletMetadata()

        this.rust = rust
        this.walletMetadata = metadata
        this.id = metadata.id

        // start fiat balance update
        mainScope.launch(Dispatchers.IO) { updateFiatBalance() }

        rust.listenForUpdates(this)
        logDebug("Initialized WalletManager from xpub")
    }

    // constructor from TapSigner
    constructor(tapSigner: TapSigner, deriveInfo: DeriveInfo, backup: ByteArray? = null) {
        val rust = RustWalletManager.tryNewFromTapSigner(tapSigner, deriveInfo, backup)
        val metadata = rust.walletMetadata()

        this.rust = rust
        this.walletMetadata = metadata
        this.id = metadata.id

        rust.listenForUpdates(this)
        logDebug("Initialized WalletManager from TapSigner")
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

    fun validateMetadata() {
        rust.validateMetadata()
    }

    suspend fun forceWalletScan() {
        rust.forceWalletScan()
    }

    suspend fun firstAddress(): AddressInfo {
        return rust.addressAt(0u)
    }

    fun amountFmt(amount: Amount): String {
        return when (walletMetadata?.selectedUnit) {
            Unit.BTC -> amount.btcString()
            Unit.SAT -> amount.satsString()
            else -> amount.satsString()
        }
    }

    fun displayAmount(amount: Amount, showUnit: Boolean = true): String {
        return rust.displayAmount(amount, showUnit)
    }

    fun amountFmtUnit(amount: Amount): String {
        return when (walletMetadata?.selectedUnit) {
            Unit.BTC -> amount.btcStringWithUnit()
            Unit.SAT -> amount.satsStringWithUnit()
            else -> amount.satsStringWithUnit()
        }
    }

    suspend fun transactionDetails(txId: TxId): TransactionDetails {
        // check cache first
        transactionDetailsCache[txId]?.let { return it }

        // fetch from rust and cache
        val details = rust.transactionDetails(txId)
        transactionDetailsCache[txId] = details
        return details
    }

    private suspend fun updateFiatBalance() {
        try {
            val fiatBal = rust.balanceInFiat()
            withContext(Dispatchers.Main) {
                fiatBalance = fiatBal
            }
        } catch (e: Exception) {
            logError("error getting fiat balance", e)
            withContext(Dispatchers.Main) {
                fiatBalance = 0.0
            }
        }
    }

    suspend fun updateWalletBalance() {
        val bal = rust.balance()
        withContext(Dispatchers.Main) {
            balance = bal
        }
        updateFiatBalance()
    }

    private fun apply(message: WalletManagerReconcileMessage) {
        when (message) {
            is WalletManagerReconcileMessage.StartedInitialFullScan -> {
                loadState = WalletLoadState.LOADING
            }

            is WalletManagerReconcileMessage.StartedExpandedFullScan -> {
                loadState = WalletLoadState.SCANNING(message.txns)
            }

            is WalletManagerReconcileMessage.AvailableTransactions -> {
                if (loadState is WalletLoadState.LOADING) {
                    loadState = WalletLoadState.SCANNING(message.txns)
                }
            }

            is WalletManagerReconcileMessage.UpdatedTransactions -> {
                loadState =
                    when (loadState) {
                        is WalletLoadState.SCANNING, is WalletLoadState.LOADING ->
                            WalletLoadState.SCANNING(message.txns)
                        is WalletLoadState.LOADED ->
                            WalletLoadState.LOADED(message.txns)
                    }
            }

            is WalletManagerReconcileMessage.ScanComplete -> {
                loadState = WalletLoadState.LOADED(message.txns)
            }

            is WalletManagerReconcileMessage.WalletBalanceChanged -> {
                balance = message.balance
                // update fiat balance in background
                mainScope.launch(Dispatchers.IO) { updateFiatBalance() }
            }

            is WalletManagerReconcileMessage.UnsignedTransactionsChanged -> {
                unsignedTransactions =
                    runCatching {
                        rust.getUnsignedTransactions()
                    }.getOrElse { emptyList() }
            }

            is WalletManagerReconcileMessage.WalletMetadataChanged -> {
                walletMetadata = message.metadata
                rust.setWalletMetadata(message.metadata)
            }

            is WalletManagerReconcileMessage.WalletScannerResponse -> {
                logDebug("walletScannerResponse: ${message.scannerResponse}")
                when (val response = message.scannerResponse) {
                    is WalletScannerResponse.FoundAddresses -> {
                        foundAddresses = response.addressTypes
                    }
                    else -> {
                        // handle other scanner response types
                    }
                }
            }

            is WalletManagerReconcileMessage.NodeConnectionFailed -> {
                errorAlert = WalletErrorAlert.NodeConnectionFailed(message.error)
                logError(message.error)
            }

            is WalletManagerReconcileMessage.WalletError -> {
                logError("WalletError: ${message.error}")
            }

            is WalletManagerReconcileMessage.UnknownError -> {
                logError("Unknown error: ${message.error}")
            }

            is WalletManagerReconcileMessage.SendFlowError -> {
                sendFlowErrorAlert = TaggedItem(message.error)
            }
        }
    }

    override fun reconcile(message: WalletManagerReconcileMessage) {
        logDebug("reconcile: $message")
        mainScope.launch { apply(message) }
    }

    override fun reconcileMany(messages: List<WalletManagerReconcileMessage>) {
        logDebug("reconcile_messages: ${messages.size} messages")
        mainScope.launch { messages.forEach { apply(it) } }
    }

    fun dispatch(action: WalletManagerAction) {
        logDebug("dispatch: $action")
        mainScope.launch(Dispatchers.IO) { rust.dispatch(action) }
    }
}

/**
 * wallet load state sealed class
 * mirrors iOS WalletLoadState enum
 */
sealed class WalletLoadState {
    data object LOADING : WalletLoadState()

    data class SCANNING(val txns: List<Transaction>) : WalletLoadState()

    data class LOADED(val txns: List<Transaction>) : WalletLoadState()
}
