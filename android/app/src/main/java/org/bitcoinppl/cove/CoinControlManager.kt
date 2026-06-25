package org.bitcoinppl.cove

import androidx.compose.runtime.Stable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.delay
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch
import org.bitcoinppl.cove_core.*
import org.bitcoinppl.cove_core.types.*
import java.io.Closeable
import java.util.concurrent.atomic.AtomicBoolean

/**
 * coin control manager - manages UTXO selection for send flow
 * ported from iOS CoinControlManager.swift
 */
@Stable
class CoinControlManager internal constructor(
    private val rust: RustCoinControlManager,
) : CoinControlManagerReconciler,
    Closeable {
    private val tag = "CoinControlManager"

    private val mainScope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)
    private val isClosed = AtomicBoolean(false)
    private val rustGuard =
        RustHandleGuard(
            ownerName = "CoinControlManager",
            handleName = "RustCoinControlManager",
            isClosed = isClosed,
        ) {
            android.util.Log.w(tag, it)
        }

    val id: WalletId = rust.id()

    var sort by mutableStateOf<CoinControlListSort?>(
        CoinControlListSort.Date(ListSortDirection.DESCENDING),
    )
        private set

    var search by mutableStateOf("")

    var totalSelected by mutableStateOf(Amount.fromSat(0u))
        private set

    var selected by mutableStateOf<Set<ULong>>(emptySet())

    var utxos by mutableStateOf<List<Utxo>>(emptyList())
        private set

    var lockStateLoadFailed by mutableStateOf(false)
        private set

    var unit by mutableStateOf(BitcoinUnit.SAT)
        private set

    private var updateSendFlowManagerTask: Job? = null

    init {
        logDebug("Initializing CoinControlManager")
        utxos = rust.utxos()
        lockStateLoadFailed = rust.lockStateLoadFailed()
        unit = rust.unit()
        rust.listenForUpdates(this)
    }

    private fun logDebug(message: String) {
        android.util.Log.d(tag, message)
    }

    private fun <T> withRust(
        block: RustCoinControlManager.() -> T,
    ): T = rustGuard.withHandle(rust, block)

    private fun <T> withRustOr(
        defaultValue: T,
        block: RustCoinControlManager.() -> T,
    ): T = rustGuard.withHandleOr(rust, defaultValue, block)

    private suspend fun <T> withRustSuspend(
        block: suspend RustCoinControlManager.() -> T,
    ): T = rustGuard.withHandleSuspend(rust, block)

    private suspend fun <T> withRustOrSuspend(
        defaultValue: T,
        block: suspend RustCoinControlManager.() -> T,
    ): T = rustGuard.withHandleOrSuspend(rust, defaultValue, block)

    /**
     * update search and dispatch notification
     */
    fun updateSearch(value: String) {
        if (search != value) {
            dispatch(CoinControlManagerAction.NotifySearchChanged(value))
        }
        search = value
    }

    /**
     * update selected utxos and dispatch notification
     */
    fun updateSelected(value: Set<ULong>) {
        val visibleIds = utxos.map { it.outpoint.hashToUint() }.toSet()
        val visibleSpendableIds = utxos.filter { it.spendable }.map { it.outpoint.hashToUint() }.toSet()
        val selectedOutsideVisibleSearch = selected.subtract(visibleIds)
        val spendableSelection = selectedOutsideVisibleSearch.union(value.intersect(visibleSpendableIds))
        selected = spendableSelection

        val hiddenSelectedOutpoints =
            withRustOr(emptyList()) {
                selectedUtxos()
            }
                .filter { selectedOutsideVisibleSearch.contains(it.outpoint.hashToUint()) }
                .map { it.outpoint }
        val visibleSelectedOutpoints =
            utxos
                .filter { it.spendable && spendableSelection.contains(it.outpoint.hashToUint()) }
                .map { it.outpoint }
        val outpoints = hiddenSelectedOutpoints + visibleSelectedOutpoints
        dispatch(CoinControlManagerAction.NotifySelectedUtxosChanged(outpoints))
    }

    /**
     * get current button presentation based on sort state
     */
    fun buttonPresentation(key: CoinControlListSortKey): ButtonPresentation? =
        withRustOr(null) {
            buttonPresentation(key)
        }

    /**
     * get button arrow icon based on sort state
     */
    fun buttonArrow(key: CoinControlListSortKey): String? =
        when (val presentation = buttonPresentation(key)) {
            is ButtonPresentation.Selected -> {
                when (presentation.v1) {
                    ListSortDirection.ASCENDING -> "arrow_upward"
                    ListSortDirection.DESCENDING -> "arrow_downward"
                }
            }
            is ButtonPresentation.NotSelected -> null
            null -> null
        }

    val totalSelectedAmount: String
        get() = displayAmount(totalSelected)

    val totalSelectedSats: Long
        get() = totalSelected.asSats().toLong()

    /**
     * called when user presses continue button
     * navigates forward to CoinControlSetAmount screen with selected UTXOs
     */
    fun continuePressed(app: AppManager) {
        val walletId = id
        val selectedUtxos =
            withRustOr(emptyList()) {
                selectedUtxos()
            }

        // navigate forward to coin control set amount screen
        val sendRoute = SendRoute.CoinControlSetAmount(walletId, selectedUtxos)
        app.pushRoute(Route.Send(sendRoute))
    }

    private fun updateSendFlowManager() {
        val sfm = AppManager.getInstance().sendFlowManager ?: return
        updateSendFlowManagerTask?.cancel()
        updateSendFlowManagerTask =
            mainScope.launch {
                delay(SEND_FLOW_UPDATE_DELAY_MS)
                if (!isActive) return@launch

                val selectedUtxos =
                    withRustOr(emptyList()) {
                        selectedUtxos()
                    }
                sfm.dispatch(SendFlowManagerAction.SetCoinControlMode(selectedUtxos))
            }
    }

    private fun apply(message: CoinControlManagerReconcileMessage) {
        when (message) {
            is CoinControlManagerReconcileMessage.UpdateSort -> {
                sort = message.v1
            }

            is CoinControlManagerReconcileMessage.ClearSort -> {
                sort = null
            }

            is CoinControlManagerReconcileMessage.UpdateUtxos -> {
                utxos = message.v1
            }

            is CoinControlManagerReconcileMessage.UpdateSearch -> {
                search = message.v1
            }

            is CoinControlManagerReconcileMessage.UpdateSelectedUtxos -> {
                updateSendFlowManager()
                selected = message.utxos.map { it.hashToUint() }.toSet()
                totalSelected = message.totalValue
            }

            is CoinControlManagerReconcileMessage.UpdateUnit -> {
                unit = message.v1
            }

            is CoinControlManagerReconcileMessage.UpdateLockStateLoadFailed -> {
                lockStateLoadFailed = message.v1
            }
        }
    }

    fun displayAmount(amount: Amount, showUnit: Boolean = true): String =
        when (unit to showUnit) {
            BitcoinUnit.BTC to true -> amount.btcStringWithUnit()
            BitcoinUnit.BTC to false -> amount.btcString()
            BitcoinUnit.SAT to true -> amount.satsStringWithUnit()
            BitcoinUnit.SAT to false -> amount.satsString()
            else -> amount.satsStringWithUnit()
        }

    suspend fun reloadLabels() {
        withRustOrSuspend(Unit) {
            reloadLabels()
        }
    }

    suspend fun setSpendability(
        outpoint: OutPoint,
        spendable: Boolean,
    ) = withRustSuspend {
        setUtxoSpendability(outpoint, spendable)
    }

    override fun reconcile(message: CoinControlManagerReconcileMessage) {
        if (isClosed.get()) return

        logDebug("reconcile: $message")
        mainScope.launch { apply(message) }
    }

    override fun reconcileMany(messages: List<CoinControlManagerReconcileMessage>) {
        if (isClosed.get()) return

        logDebug("reconcile_messages: ${messages.size} messages")
        mainScope.launch { messages.forEach { apply(it) } }
    }

    fun dispatch(action: CoinControlManagerAction) {
        logDebug("dispatch: $action")
        mainScope.launch(Dispatchers.IO) {
            withRustOr(Unit) {
                dispatch(action)
            }
        }
    }

    override fun close() {
        rustGuard.closeOnce {
            logDebug("Closing CoinControlManager")
            updateSendFlowManagerTask?.cancel()
            mainScope.cancel()
            rust.close()
        }
    }

    companion object {

        /**
         * delay before propagating coin control selection to SendFlowManager
         *
         * batches rapid UTXO selection changes into a single update
         * prevents excessive dispatch and recomputation during multi-select
         */
        private const val SEND_FLOW_UPDATE_DELAY_MS = 100L
    }
}
