package org.bitcoinppl.cove

import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import org.bitcoinppl.cove_core.*
import org.bitcoinppl.cove_core.types.*
import java.io.Closeable
import java.util.concurrent.atomic.AtomicBoolean

/**
 * send flow presenter - manages UI state for send flow screens
 * ported from iOS SendFlowPresenter.swift
 */
class SendFlowPresenter(
    val app: AppManager,
    val manager: WalletManager,
) : Closeable {
    // TODO: use when implementing alert dialogs - prevents alerts from reappearing during dismissal animations
    // see iOS showingAlert at SendFlowPresenter.swift:38 for usage pattern
    private var disappearing: Boolean = false

    private val mainScope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)
    private val isClosed = AtomicBoolean(false)

    var focusField by mutableStateOf<SetAmountFocusField?>(null)
    var sheetState by mutableStateOf<TaggedItem<SheetState>?>(null)
    var alertState by mutableStateOf<TaggedItem<SendFlowAlertState>?>(null)

    var lastWorkingFeeRate by mutableStateOf<Float?>(null)
    var erroredFeeRate by mutableStateOf<Float?>(null)

    /**
     * sheet states for send flow
     */
    sealed class SheetState {
        data object Qr : SheetState()

        data object Fee : SheetState()

        override fun equals(other: Any?): Boolean {
            if (this === other) return true
            if (other == null) return false
            return this::class == other::class
        }

        override fun hashCode(): Int {
            return this::class.hashCode()
        }
    }

    fun setDisappearing() {
        disappearing = true
        mainScope.launch {
            delay(500)
            disappearing = false
        }
    }

    /**
     * get alert title based on alert state
     */
    fun alertTitle(): String {
        return when (val state = alertState?.item) {
            is SendFlowAlertState.Error -> errorAlertTitle(state.v1)
            is SendFlowAlertState.General -> state.title
            null -> ""
        }
    }

    private fun errorAlertTitle(error: SendFlowException): String {
        return when (error) {
            is SendFlowException.EmptyAddress,
            is SendFlowException.InvalidAddress,
            is SendFlowException.WrongNetwork,
            -> "Invalid Address"

            is SendFlowException.InvalidNumber,
            is SendFlowException.ZeroAmount,
            -> "Invalid Amount"

            is SendFlowException.InsufficientFunds,
            is SendFlowException.NoBalance,
            -> "Insufficient Funds"

            is SendFlowException.SendAmountToLow -> "Send Amount Too Low"
            is SendFlowException.UnableToGetFeeRate -> "Unable to get fee rate"
            is SendFlowException.UnableToBuildTxn -> "Unable to build transaction"
            is SendFlowException.UnableToGetMaxSend -> "Unable to get max send"
            is SendFlowException.UnableToSaveUnsignedTransaction -> "Unable to Save Unsigned Transaction"
            is SendFlowException.WalletManager -> "Error"
            is SendFlowException.UnableToGetFeeDetails -> "Fee Details Error"
        }
    }

    /**
     * get alert message text based on alert state
     */
    fun alertMessage(): String {
        return when (val state = alertState?.item) {
            is SendFlowAlertState.Error -> errorAlertMessage(state.v1)
            is SendFlowAlertState.General -> state.message
            null -> ""
        }
    }

    private fun errorAlertMessage(error: SendFlowException): String {
        return when (error) {
            is SendFlowException.EmptyAddress ->
                "Please enter an address"

            is SendFlowException.InvalidNumber ->
                "Please enter a valid number for the amount to send"

            is SendFlowException.ZeroAmount ->
                "Can't send an empty transaction. Please enter a valid amount"

            is SendFlowException.NoBalance ->
                "You do not have any bitcoin in your wallet. Please add some to send a transaction"

            is SendFlowException.InvalidAddress ->
                "The address ${error.v1} is invalid"

            is SendFlowException.WrongNetwork ->
                "The address ${error.address} is on the wrong network, it is for ${error.validFor}. You are on ${error.current}"

            is SendFlowException.InsufficientFunds ->
                "You do not have enough bitcoin in your wallet to cover the amount plus fees"

            is SendFlowException.SendAmountToLow ->
                "Send amount is too low. Please send at least 5000 sats"

            is SendFlowException.UnableToGetFeeRate ->
                "Are you connected to the internet?"

            is SendFlowException.WalletManager ->
                error.v1.message ?: "Wallet Manager Error"

            is SendFlowException.UnableToGetFeeDetails ->
                error.v1

            is SendFlowException.UnableToBuildTxn ->
                error.v1

            is SendFlowException.UnableToGetMaxSend ->
                error.v1

            is SendFlowException.UnableToSaveUnsignedTransaction ->
                error.v1
        }
    }

    /**
     * get alert button action based on error type
     */
    fun alertButtonAction(): (() -> Unit)? {
        return when (val state = alertState?.item) {
            is SendFlowAlertState.Error -> errorAlertButtonAction(state.v1)
            is SendFlowAlertState.General -> {
                { alertState = null }
            }
            null -> null
        }
    }

    private fun errorAlertButtonAction(error: SendFlowException): () -> Unit {
        return when (error) {
            is SendFlowException.EmptyAddress,
            is SendFlowException.WrongNetwork,
            is SendFlowException.InvalidAddress,
            -> {
                {
                    alertState = null
                    focusField = SetAmountFocusField.ADDRESS
                }
            }

            is SendFlowException.NoBalance -> {
                {
                    alertState = null
                    app.popRoute()
                }
            }

            is SendFlowException.InvalidNumber,
            is SendFlowException.InsufficientFunds,
            is SendFlowException.SendAmountToLow,
            is SendFlowException.ZeroAmount,
            is SendFlowException.WalletManager,
            is SendFlowException.UnableToGetFeeDetails,
            is SendFlowException.UnableToGetFeeRate,
            is SendFlowException.UnableToBuildTxn,
            is SendFlowException.UnableToSaveUnsignedTransaction,
            is SendFlowException.UnableToGetMaxSend,
            -> {
                {
                    focusField = SetAmountFocusField.AMOUNT
                    alertState = null
                }
            }
        }
    }

    override fun close() {
        if (!isClosed.compareAndSet(false, true)) return
        mainScope.cancel()
    }
}
