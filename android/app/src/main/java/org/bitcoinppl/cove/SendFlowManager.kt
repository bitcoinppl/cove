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
 * send flow manager - manages send transaction flow state
 * ported from iOS SendFlowManager.swift
 */
@Stable
class SendFlowManager(
    internal val rust: RustSendFlowManager,
    var presenter: SendFlowPresenter,
) : SendFlowManagerReconciler,
    Closeable {
    private val tag = "SendFlowManager"

    // Scope for UI-bound work; reconcile and UI updates run on Main
    private val mainScope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)
    private val ioScope = CoroutineScope(SupervisorJob() + Dispatchers.IO)
    private val isClosed = AtomicBoolean(false)

    val id: WalletId = rust.walletId()

    // user input state
    var enteringBtcAmount by mutableStateOf("")
        private set

    var enteringFiatAmount by mutableStateOf("")
        private set

    private var _enteringAddress by mutableStateOf("")

    // validated state
    var address by mutableStateOf<Address?>(null)

    var amount by mutableStateOf<Amount?>(null)

    var fiatAmount by mutableStateOf<Double?>(null)
        private set

    // fee state
    var selectedFeeRate by mutableStateOf<FeeRateOptionWithTotalFee?>(null)
        private set

    var feeRateOptions by mutableStateOf<FeeRateOptionsWithTotalFee?>(null)
        private set

    var maxSelected by mutableStateOf<Amount?>(null)
        private set

    // presenting strings
    var sendAmountFiat by mutableStateOf("")
        private set

    var sendAmountBtc by mutableStateOf("")
        private set

    var totalSpentInFiat by mutableStateOf("")
        private set

    var totalSpentInBtc by mutableStateOf("")
        private set

    var totalFeeString by mutableStateOf<String?>(null)
        private set

    // debounce task
    private var debouncedTask: Job? = null

    init {
        logDebug("Initializing SendFlowManager for $id")
        enteringFiatAmount = rust.enteringFiatAmount()
        sendAmountFiat = rust.sendAmountFiat()
        sendAmountBtc = rust.sendAmountBtc()
        totalSpentInFiat = rust.totalSpentInFiat()
        totalSpentInBtc = rust.totalSpentInBtc()
        totalFeeString = rust.totalFeeString()

        rust.listenForUpdates(this)
    }

    private fun logDebug(message: String) {
        android.util.Log.d(tag, message)
    }

    private fun logWarn(message: String) {
        android.util.Log.w(tag, message)
    }

    /**
     * get/set entering address with dispatch
     */
    var enteringAddress: String
        get() = _enteringAddress
        set(value) {
            _enteringAddress = value
            dispatch(SendFlowManagerAction.NotifyEnteringAddressChanged(value))
        }

    /**
     * update entering BTC amount with debounced dispatch
     * only dispatches if value actually changed (matches iOS pattern)
     */
    fun updateEnteringBtcAmount(value: String) {
        if (enteringBtcAmount != value) {
            enteringBtcAmount = value
            debouncedDispatch(SendFlowManagerAction.NotifyEnteringBtcAmountChanged(value))
        }
    }

    /**
     * update entering fiat amount with debounced dispatch
     * only dispatches if value actually changed (matches iOS pattern)
     */
    fun updateEnteringFiatAmount(value: String) {
        if (enteringFiatAmount != value) {
            enteringFiatAmount = value
            debouncedDispatch(SendFlowManagerAction.NotifyEnteringFiatAmountChanged(value))
        }
    }

    /**
     * validate entire send flow
     */
    fun validate(displayAlert: Boolean = false): Boolean =
        validateAmount(displayAlert) &&
            validateAddress(displayAlert) &&
            validateFeePercentage(displayAlert)

    fun validateAddress(displayAlert: Boolean = false): Boolean = rust.validateAddress(displayAlert)

    fun validateAmount(displayAlert: Boolean = false): Boolean = rust.validateAmount(displayAlert)

    fun validateFeePercentage(displayAlert: Boolean = false): Boolean = rust.validateFeePercentage(displayAlert)

    fun updateAddress(address: Address) {
        _enteringAddress = address.string()
        this.address = address
        dispatch(SendFlowManagerAction.NotifyAddressChanged(address))
    }

    fun updateAmount(amount: Amount) {
        this.amount = amount
        dispatch(SendFlowManagerAction.NotifyAmountChanged(amount))
    }

    fun refreshPresenters() {
        totalSpentInFiat = rust.totalSpentInFiat()
        totalSpentInBtc = rust.totalSpentInBtc()
        totalFeeString = rust.totalFeeString()
        sendAmountBtc = rust.sendAmountBtc()
        sendAmountFiat = rust.sendAmountFiat()
    }

    suspend fun getNewCustomFeeRateWithTotal(
        feeRate: FeeRate,
        feeSpeed: FeeSpeed,
    ): FeeRateOptionWithTotalFee = rust.getCustomFeeOption(feeRate, feeSpeed)

    private fun apply(message: SendFlowManagerReconcileMessage) {
        when (message) {
            is SendFlowManagerReconcileMessage.UpdateAmountFiat -> {
                fiatAmount = message.v1
            }

            is SendFlowManagerReconcileMessage.UpdateAmountSats -> {
                refreshPresenters()
                amount = Amount.fromSat(message.v1)
            }

            is SendFlowManagerReconcileMessage.UpdateFeeRateOptions -> {
                refreshPresenters()
                feeRateOptions = message.v1
            }

            is SendFlowManagerReconcileMessage.UpdateAddress -> {
                address = message.v1
            }

            is SendFlowManagerReconcileMessage.UpdateEnteringBtcAmount -> {
                enteringBtcAmount = message.v1
            }

            is SendFlowManagerReconcileMessage.UpdateEnteringAddress -> {
                _enteringAddress = message.v1
            }

            is SendFlowManagerReconcileMessage.UpdateEnteringFiatAmount -> {
                enteringFiatAmount = message.v1
            }

            is SendFlowManagerReconcileMessage.UpdateSelectedFeeRate -> {
                refreshPresenters()
                selectedFeeRate = message.v1
            }

            is SendFlowManagerReconcileMessage.UpdateFocusField -> {
                presenter.focusField = message.v1
            }

            is SendFlowManagerReconcileMessage.SetAlert -> {
                logWarn("setAlert: ${message.v1}")

                // capture previous state before modifying
                val hadSheet = presenter.sheetState != null
                val hadAlert = presenter.alertState != null

                presenter.alertState = TaggedItem(message.v1)

                // handle alert/sheet conflict - delay only if there was a previous conflict
                if (hadSheet || hadAlert) {
                    presenter.alertState = null
                    presenter.sheetState = null
                    mainScope.launch {
                        delay(600)
                        presenter.alertState = TaggedItem(message.v1)
                    }
                }
            }

            is SendFlowManagerReconcileMessage.ClearAlert -> {
                presenter.alertState = null
            }

            is SendFlowManagerReconcileMessage.SetMaxSelected -> {
                maxSelected = message.v1
            }

            is SendFlowManagerReconcileMessage.UnsetMaxSelected -> {
                maxSelected = null
            }

            is SendFlowManagerReconcileMessage.RefreshPresenters -> {
                refreshPresenters()
            }
        }
    }

    override fun reconcile(message: SendFlowManagerReconcileMessage) {
        logDebug("reconcile: $message")
        ioScope.launch {
            mainScope.launch { apply(message) }
        }
    }

    override fun reconcileMany(messages: List<SendFlowManagerReconcileMessage>) {
        logDebug("reconcile_messages: ${messages.size} messages")
        ioScope.launch {
            mainScope.launch { messages.forEach { apply(it) } }
        }
    }

    fun dispatch(action: SendFlowManagerAction) {
        logDebug("dispatch: $action")
        mainScope.launch(Dispatchers.IO) { rust.dispatch(action) }
    }

    /**
     * dispatch with debouncing for high-frequency updates
     */
    fun debouncedDispatch(
        action: SendFlowManagerAction,
        debounceDelayMs: Long = 66,
    ) {
        debouncedTask?.cancel()
        debouncedTask = null

        if (debounceDelayMs <= 0) {
            dispatch(action)
            return
        }

        debouncedTask =
            mainScope.launch {
                delay(debounceDelayMs)
                if (!isActive) return@launch
                dispatch(action)
            }
    }

    override fun close() {
        if (!isClosed.compareAndSet(false, true)) return
        logDebug("Closing SendFlowManager for $id")
        debouncedTask?.cancel()
        ioScope.cancel()
        mainScope.cancel() // stop callbacks into Rust
        rust.close() // free Rust Arc
    }
}
