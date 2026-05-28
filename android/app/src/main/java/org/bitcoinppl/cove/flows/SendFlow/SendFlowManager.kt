package org.bitcoinppl.cove.flows.SendFlow

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
import org.bitcoinppl.cove.TaggedItem
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
    private val rust: RustSendFlowManager,
    var presenter: SendFlowPresenter,
) : SendFlowManagerReconciler,
    Closeable {
    private val tag = "SendFlowManager"

    // Scope for UI-bound work; reconcile and UI updates run on Main
    private val mainScope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)
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
    var feeSelection by mutableStateOf<FeeSelection?>(null)
        private set

    val selectedFeeRate: FeeRateOptionWithTotalFee?
        get() = feeSelection?.selected

    val feeRateOptions: FeeRateOptionsWithTotalFee?
        get() = feeSelection?.options

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

    private fun markClosedAfterDestroyedHandle(
        callName: String,
        error: IllegalStateException,
    ) {
        isClosed.set(true)
        logWarn("$callName skipped because RustSendFlowManager is closed: ${error.message}")
    }

    private inline fun <T> withRustOr(
        defaultValue: T,
        callName: String,
        block: RustSendFlowManager.() -> T,
    ): T {
        if (isClosed.get()) return defaultValue

        return try {
            rust.block()
        } catch (e: IllegalStateException) {
            markClosedAfterDestroyedHandle(callName, e)
            defaultValue
        }
    }

    private suspend inline fun <T> withRustOrSuspend(
        defaultValue: T,
        callName: String,
        crossinline block: suspend RustSendFlowManager.() -> T,
    ): T {
        if (isClosed.get()) return defaultValue

        return try {
            rust.block()
        } catch (e: IllegalStateException) {
            markClosedAfterDestroyedHandle(callName, e)
            defaultValue
        }
    }

    /**
     * get/set entering address with dispatch
     */
    var enteringAddress: String
        get() = _enteringAddress
        set(value) {
            if (isClosed.get()) return
            _enteringAddress = value
            dispatch(SendFlowManagerAction.NotifyEnteringAddressChanged(value))
        }

    /**
     * update entering BTC amount with debounced dispatch
     * only dispatches if value actually changed (matches iOS pattern)
     */
    fun updateEnteringBtcAmount(value: String) {
        if (isClosed.get()) return
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
        if (isClosed.get()) return
        if (enteringFiatAmount != value) {
            enteringFiatAmount = value
            debouncedDispatch(SendFlowManagerAction.NotifyEnteringFiatAmountChanged(value))
        }
    }

    /**
     * validate entire send flow
     */
    fun validate(displayAlert: Boolean = false): Boolean {
        if (isClosed.get()) return false
        return validateAmount(displayAlert) &&
                validateAddress(displayAlert) &&
                validateFeePercentage(displayAlert)
    }

    suspend fun waitForInit(): Boolean =
        withRustOrSuspend(false, "waitForInit") {
            waitForInit()
        }

    fun amountExceedsBalance(): Boolean =
        withRustOr(false, "amountExceedsBalance") {
            amountExceedsBalance()
        }

    fun currentAmount(): Amount? =
        withRustOr(null, "amount") {
            amount()
        }

    fun maxSendMinusFees(): Amount? =
        withRustOr(null, "maxSendMinusFees") {
            maxSendMinusFees()
        }

    fun maxSendMinusFeesAndSmallUtxo(): Amount? =
        withRustOr(null, "maxSendMinusFeesAndSmallUtxo") {
            maxSendMinusFeesAndSmallUtxo()
        }

    fun sanitizeBtcEnteringAmount(
        oldValue: String,
        newValue: String,
    ): String? =
        withRustOr(null, "sanitizeBtcEnteringAmount") {
            sanitizeBtcEnteringAmount(oldValue, newValue)
        }

    fun sanitizeFiatEnteringAmount(
        oldValue: String,
        newValue: String,
    ): String? =
        withRustOr(null, "sanitizeFiatEnteringAmount") {
            sanitizeFiatEnteringAmount(oldValue, newValue)
        }

    fun validateAddress(displayAlert: Boolean = false): Boolean =
        withRustOr(false, "validateAddress") {
            validateAddress(displayAlert)
        }

    fun validateAmount(displayAlert: Boolean = false): Boolean =
        withRustOr(false, "validateAmount") {
            validateAmount(displayAlert)
        }

    fun validateFeePercentage(displayAlert: Boolean = false): Boolean =
        withRustOr(false, "validateFeePercentage") {
            validateFeePercentage(displayAlert)
        }

    fun updateAddress(address: Address) {
        if (isClosed.get()) return
        _enteringAddress = address.unformatted()
        this.address = address
        dispatch(SendFlowManagerAction.NotifyAddressChanged(address))
    }

    fun updateAmount(amount: Amount) {
        if (isClosed.get()) return
        this.amount = amount
        dispatch(SendFlowManagerAction.NotifyAmountChanged(amount))
    }

    fun refreshPresenters() {
        totalSpentInFiat =
            withRustOr(totalSpentInFiat, "totalSpentInFiat") {
                totalSpentInFiat()
            }
        totalSpentInBtc =
            withRustOr(totalSpentInBtc, "totalSpentInBtc") {
                totalSpentInBtc()
            }
        totalFeeString =
            withRustOr(totalFeeString, "totalFeeString") {
                totalFeeString()
            }
        sendAmountBtc =
            withRustOr(sendAmountBtc, "sendAmountBtc") {
                sendAmountBtc()
            }
        sendAmountFiat =
            withRustOr(sendAmountFiat, "sendAmountFiat") {
                sendAmountFiat()
            }
    }

    suspend fun getNewCustomFeeRateWithTotal(
        feeRate: FeeRate,
        feeSpeed: FeeSpeed,
    ): FeeRateOptionWithTotalFee {
        if (isClosed.get()) {
            throw IllegalStateException("SendFlowManager is closed")
        }

        return try {
            rust.getCustomFeeOption(feeRate, feeSpeed)
        } catch (e: IllegalStateException) {
            markClosedAfterDestroyedHandle("getCustomFeeOption", e)
            throw e
        }
    }

    private fun apply(message: SendFlowManagerReconcileMessage) {
        when (message) {
            is SendFlowManagerReconcileMessage.UpdateAmountFiat -> {
                fiatAmount = message.v1
            }

            is SendFlowManagerReconcileMessage.UpdateAmountSats -> {
                refreshPresenters()
                amount = Amount.fromSat(message.v1)
            }

            is SendFlowManagerReconcileMessage.UpdateFeeSelection -> {
                refreshPresenters()
                feeSelection = message.v1
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
                        delay(ALERT_PRESENTATION_DELAY_MS)
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
        mainScope.launch { apply(message) }
    }

    override fun reconcileMany(messages: List<SendFlowManagerReconcileMessage>) {
        logDebug("reconcile_messages: ${messages.size} messages")
        mainScope.launch { messages.forEach { apply(it) } }
    }

    fun dispatch(action: SendFlowManagerAction) {
        if (isClosed.get()) return
        logDebug("dispatch: $action")
        mainScope.launch(Dispatchers.IO) {
            withRustOr(Unit, "dispatch") {
                dispatch(action)
            }
        }
    }

    /**
     * dispatch with debouncing for high-frequency updates
     */
    fun debouncedDispatch(
        action: SendFlowManagerAction,
        debounceDelayMs: Long = DEFAULT_DEBOUNCE_MS,
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
        debouncedTask = null
        mainScope.cancel()
        rust.close()
    }

    companion object {

        /**
         * delay before showing alert when another modal (sheet/alert) was visible
         *
         * allows previous modal dismiss animation to complete before presenting a new alert
         * material3 bottom sheet dismiss animation ≈ 500ms → extra buffer prevents flicker
         */
        private const val ALERT_PRESENTATION_DELAY_MS = 600L

        /**
         * default debounce for amount input
         * ~60fps target = 16.67ms per frame, 66ms = ~4 frames
         * balances responsiveness vs rust bridge overhead
         */
        private const val DEFAULT_DEBOUNCE_MS = 66L
    }
}
