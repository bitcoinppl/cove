package org.bitcoinppl.cove

import androidx.compose.runtime.Stable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.graphics.Color
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
class CoinControlManager(
    val rust: RustCoinControlManager,
) : CoinControlManagerReconciler,
    Closeable {
    private val tag = "CoinControlManager"

    private val mainScope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)
    private val isClosed = AtomicBoolean(false)

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

    var unit by mutableStateOf(BitcoinUnit.SAT)
        private set

    private var updateSendFlowManagerTask: Job? = null

    init {
        logDebug("Initializing CoinControlManager")
        utxos = rust.utxos()
        unit = rust.unit()
        rust.listenForUpdates(this)
    }

    private fun logDebug(message: String) {
        android.util.Log.d(tag, message)
    }

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
        selected = value
        val outpoints = utxos.filter { value.contains(it.outpoint.hashToUint()) }.map { it.outpoint }
        dispatch(CoinControlManagerAction.NotifySelectedUtxosChanged(outpoints))
    }

    /**
     * get button color based on sort state
     */
    fun buttonColor(key: CoinControlListSortKey): Color =
        when (rust.buttonPresentation(key)) {
            is ButtonPresentation.NotSelected -> Color(0xFFD1D1D6) // systemGray5 equivalent
            is ButtonPresentation.Selected -> Color(0xFF007AFF) // iOS blue
        }

    /**
     * get button text color based on sort state
     */
    fun buttonTextColor(key: CoinControlListSortKey): Color =
        when (rust.buttonPresentation(key)) {
            is ButtonPresentation.NotSelected -> Color(0xFF8E8E93).copy(alpha = 0.6f) // secondary with opacity
            is ButtonPresentation.Selected -> Color.White
        }

    /**
     * get button arrow icon based on sort state
     */
    fun buttonArrow(key: CoinControlListSortKey): String? =
        when (val presentation = rust.buttonPresentation(key)) {
            is ButtonPresentation.Selected -> {
                when (presentation.v1) {
                    ListSortDirection.ASCENDING -> "arrow_upward"
                    ListSortDirection.DESCENDING -> "arrow_downward"
                }
            }
            is ButtonPresentation.NotSelected -> null
        }

    val totalSelectedAmount: String
        get() = displayAmount(totalSelected)

    val totalSelectedSats: Int
        get() = totalSelected.asSats().toInt()

    /**
     * called when user presses continue button
     * navigates forward to CoinControlSetAmount screen with selected UTXOs
     */
    fun continuePressed(app: AppManager) {
        val walletId = rust.id()
        val selectedUtxos = utxos.filter { selected.contains(it.outpoint.hashToUint()) }

        // navigate forward to coin control set amount screen
        val sendRoute = SendRoute.CoinControlSetAmount(walletId, selectedUtxos)
        app.pushRoute(Route.Send(sendRoute))
    }

    private fun updateSendFlowManager() {
        val sfm = AppManager.getInstance().sendFlowManager ?: return
        updateSendFlowManagerTask?.cancel()
        updateSendFlowManagerTask =
            mainScope.launch {
                delay(100)
                if (!isActive) return@launch
                val selectedUtxos = utxos.filter { selected.contains(it.outpoint.hashToUint()) }
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

            is CoinControlManagerReconcileMessage.UpdateTotalSelectedAmount -> {
                updateSendFlowManager()
                totalSelected = message.v1
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

    override fun reconcile(message: CoinControlManagerReconcileMessage) {
        logDebug("reconcile: $message")
        mainScope.launch { apply(message) }
    }

    override fun reconcileMany(messages: List<CoinControlManagerReconcileMessage>) {
        logDebug("reconcile_messages: ${messages.size} messages")
        mainScope.launch { messages.forEach { apply(it) } }
    }

    fun dispatch(action: CoinControlManagerAction) {
        logDebug("dispatch: $action")
        mainScope.launch(Dispatchers.IO) { rust.dispatch(action) }
    }

    override fun close() {
        if (!isClosed.compareAndSet(false, true)) return
        logDebug("Closing CoinControlManager")
        updateSendFlowManagerTask?.cancel()
        mainScope.cancel()
        rust.close()
    }
}
