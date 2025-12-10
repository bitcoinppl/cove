package org.bitcoinppl.cove

import androidx.compose.runtime.Stable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.graphics.Color
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import org.bitcoinppl.cove.utils.toComposeColor
import org.bitcoinppl.cove_core.*
import org.bitcoinppl.cove_core.tapcard.TapSigner
import org.bitcoinppl.cove_core.types.*
import java.io.Closeable
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.atomic.AtomicBoolean

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
    private val transactionDetailsCache = ConcurrentHashMap<TxId, TransactionDetails>()

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
            when (loadState) {
                is WalletLoadState.LOADING -> false
                is WalletLoadState.SCANNING -> (loadState as WalletLoadState.SCANNING).txns.isNotEmpty()
                is WalletLoadState.LOADED -> (loadState as WalletLoadState.LOADED).txns.isNotEmpty()
            }

    val isVerified: Boolean
        get() = walletMetadata?.verified ?: false

    val accentColor: Color
        get() = walletMetadata?.color?.toComposeColor() ?: Color.Blue

    // private constructor - use companion factory methods
    private constructor(
        walletId: WalletId,
        rustManager: RustWalletManager,
        metadata: WalletMetadata,
    ) {
        this.id = walletId
        this.rust = rustManager
        this.walletMetadata = metadata
        this.unsignedTransactions = runCatching { rustManager.getUnsignedTransactions() }.getOrElse { emptyList() }

        // start fiat balance update
        mainScope.launch(Dispatchers.IO) { updateFiatBalance() }

        rustManager.listenForUpdates(this)
    }

    companion object {
        // create from wallet ID
        operator fun invoke(id: WalletId): WalletManager {
            val rust = RustWalletManager(id)
            val metadata = rust.walletMetadata()
            android.util.Log.d("WalletManager", "Initialized WalletManager for $id")
            return WalletManager(id, rust, metadata)
        }

        // create from xpub
        fun fromXpub(xpub: String): WalletManager {
            val rust = RustWalletManager.tryNewFromXpub(xpub)
            val metadata = rust.walletMetadata()
            android.util.Log.d("WalletManager", "Initialized WalletManager from xpub")
            return WalletManager(metadata.id, rust, metadata)
        }

        // create from TapSigner
        fun fromTapSigner(tapSigner: TapSigner, deriveInfo: DeriveInfo, backup: ByteArray? = null): WalletManager {
            val rust = RustWalletManager.tryNewFromTapSigner(tapSigner, deriveInfo, backup)
            val metadata = rust.walletMetadata()
            android.util.Log.d("WalletManager", "Initialized WalletManager from TapSigner")
            return WalletManager(metadata.id, rust, metadata)
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

    fun validateMetadata() {
        rust.validateMetadata()
    }

    suspend fun forceWalletScan() {
        rust.forceWalletScan()
    }

    fun setScanning() {
        val currentTxns = when (val state = loadState) {
            is WalletLoadState.LOADED -> state.txns
            is WalletLoadState.SCANNING -> state.txns
            else -> emptyList()
        }
        loadState = WalletLoadState.SCANNING(currentTxns)
    }

    suspend fun firstAddress(): AddressInfo = rust.addressAt(0u)

    fun amountFmt(amount: Amount): String =
        when (walletMetadata?.selectedUnit) {
            BitcoinUnit.BTC -> amount.btcString()
            BitcoinUnit.SAT -> amount.satsString()
            else -> amount.satsString()
        }

    fun displayAmount(amount: Amount, showUnit: Boolean = true): String = rust.displayAmount(amount, showUnit)

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
        val details = rust.transactionDetails(txId)
        transactionDetailsCache[txId] = details
        return details
    }

    fun updateTransactionDetailsCache(txId: TxId, details: TransactionDetails) {
        transactionDetailsCache[txId] = details
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
                loadState = WalletLoadState.SCANNING(message.v1)
            }

            is WalletManagerReconcileMessage.AvailableTransactions -> {
                if (loadState is WalletLoadState.LOADING) {
                    loadState = WalletLoadState.SCANNING(message.v1)
                }
            }

            is WalletManagerReconcileMessage.UpdatedTransactions -> {
                loadState =
                    when (loadState) {
                        is WalletLoadState.SCANNING, is WalletLoadState.LOADING ->
                            WalletLoadState.SCANNING(message.v1)
                        is WalletLoadState.LOADED ->
                            WalletLoadState.LOADED(message.v1)
                    }
            }

            is WalletManagerReconcileMessage.ScanComplete -> {
                loadState = WalletLoadState.LOADED(message.v1)
            }

            is WalletManagerReconcileMessage.WalletBalanceChanged -> {
                balance = message.v1
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
        }
    }

    override fun reconcile(message: WalletManagerReconcileMessage) {
        logDebug("reconcile: $message")
        ioScope.launch {
            mainScope.launch { apply(message) }
        }
    }

    override fun reconcileMany(messages: List<WalletManagerReconcileMessage>) {
        logDebug("reconcile_messages: ${messages.size} messages")
        ioScope.launch {
            mainScope.launch { messages.forEach { apply(it) } }
        }
    }

    fun dispatch(action: WalletManagerAction) {
        logDebug("dispatch: $action")
        mainScope.launch(Dispatchers.IO) { rust.dispatch(action) }
    }

    private fun persistWalletMetadata(metadata: WalletMetadata) {
        ioScope.launch { rust.setWalletMetadata(metadata) }
    }

    override fun close() {
        if (!isClosed.compareAndSet(false, true)) return
        logDebug("Closing WalletManager for $id")
        ioScope.cancel()
        mainScope.cancel()
        rust.close()
    }
}

/**
 * wallet load state sealed class
 * mirrors iOS WalletLoadState enum
 */
sealed class WalletLoadState {
    data object LOADING : WalletLoadState()

    data class SCANNING(
        val txns: List<Transaction>,
    ) : WalletLoadState()

    data class LOADED(
        val txns: List<Transaction>,
    ) : WalletLoadState()
}
