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
import kotlinx.coroutines.launch
import java.io.Closeable
import java.util.concurrent.atomic.AtomicBoolean

/**
 * coin control manager - manages UTXO selection for send flow
 * ported from iOS CoinControlManager.swift
 */
@Stable
class CoinControlManager(
    val rust: RustCoinControlManager,
) : CoinControlManagerReconciler, Closeable {
    private val tag = "CoinControlManager"

    private val mainScope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)
    private val isClosed = AtomicBoolean(false)

    private var sort by mutableStateOf<CoinControlListSort?>(
        CoinControlListSort.Date(SortOrder.DESCENDING),
    )

    var search by mutableStateOf("")
        private set

    var totalSelected by mutableStateOf(Amount.fromSat(0u))
        private set

    var selected by mutableStateOf<Set<String>>(emptySet())
        private set

    var utxos by mutableStateOf<List<Utxo>>(emptyList())
        private set

    var unit by mutableStateOf(Unit.SAT)
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
     * custom setter for search that dispatches notification
     */
    fun setSearch(value: String) {
        if (search != value) {
            dispatch(CoinControlManagerAction.NotifySearchChanged(value))
        }
        search = value
    }

    /**
     * custom setter for selected that dispatches notification
     */
    fun setSelected(value: Set<String>) {
        selected = value
        dispatch(CoinControlManagerAction.NotifySelectedUtxosChanged(value.toList()))
    }

    /**
     * get button color based on sort state
     */
    fun buttonColor(key: CoinControlListSortKey): Color {
        return when (rust.buttonPresentation(key)) {
            is ButtonPresentation.NotSelected -> Color(0xFFD1D1D6) // systemGray5 equivalent
            is ButtonPresentation.Selected -> Color(0xFF007AFF) // iOS blue
        }
    }

    /**
     * get button text color based on sort state
     */
    fun buttonTextColor(key: CoinControlListSortKey): Color {
        return when (rust.buttonPresentation(key)) {
            is ButtonPresentation.NotSelected -> Color(0xFF8E8E93).copy(alpha = 0.6f) // secondary with opacity
            is ButtonPresentation.Selected -> Color.White
        }
    }

    /**
     * get button arrow icon based on sort state
     */
    fun buttonArrow(key: CoinControlListSortKey): String? {
        return when (val presentation = rust.buttonPresentation(key)) {
            is ButtonPresentation.Selected -> {
                when (presentation.order) {
                    SortOrder.ASCENDING -> "arrow_upward"
                    SortOrder.DESCENDING -> "arrow_downward"
                }
            }
            is ButtonPresentation.NotSelected -> null
        }
    }

    val totalSelectedAmount: String
        get() = displayAmount(totalSelected)

    val totalSelectedSats: Int
        get() = totalSelected.asSats().toInt()

    /**
     * called when user presses continue button
     * applies selection to SendFlowManager
     */
    fun continuePressed() {
        val sfm = AppManager.getInstance().sendFlowManager ?: return
        updateSendFlowManagerTask?.cancel()
        updateSendFlowManagerTask = null

        val selectedUtxos = utxos.filter { selected.contains(it.id) }
        sfm.dispatch(SendFlowManagerAction.SetCoinControlMode(selectedUtxos))
    }

    private fun updateSendFlowManager() {
        val sfm = AppManager.getInstance().sendFlowManager ?: return
        updateSendFlowManagerTask?.cancel()
        updateSendFlowManagerTask =
            mainScope.launch {
                delay(100)
                if (!isActive) return@launch
                val selectedUtxos = utxos.filter { selected.contains(it.id) }
                sfm.dispatch(SendFlowManagerAction.SetCoinControlMode(selectedUtxos))
            }
    }

    private fun apply(message: CoinControlManagerReconcileMessage) {
        when (message) {
            is CoinControlManagerReconcileMessage.UpdateSort -> {
                sort = message.sort
            }

            is CoinControlManagerReconcileMessage.ClearSort -> {
                sort = null
            }

            is CoinControlManagerReconcileMessage.UpdateUtxos -> {
                utxos = message.utxos
            }

            is CoinControlManagerReconcileMessage.UpdateSearch -> {
                search = message.search
            }

            is CoinControlManagerReconcileMessage.UpdateSelectedUtxos -> {
                updateSendFlowManager()
                selected = message.utxos.toSet()
                totalSelected = message.totalSelected
            }

            is CoinControlManagerReconcileMessage.UpdateUnit -> {
                unit = message.unit
            }

            is CoinControlManagerReconcileMessage.UpdateTotalSelectedAmount -> {
                updateSendFlowManager()
                totalSelected = message.amount
            }
        }
    }

    fun displayAmount(amount: Amount, showUnit: Boolean = true): String {
        return when (unit to showUnit) {
            Unit.BTC to true -> amount.btcStringWithUnit()
            Unit.BTC to false -> amount.btcString()
            Unit.SAT to true -> amount.satsStringWithUnit()
            Unit.SAT to false -> amount.satsString()
            else -> amount.satsStringWithUnit()
        }
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
        mainScope.cancel() // stop callbacks into Rust
        rust.close() // free Rust Arc
    }
}
