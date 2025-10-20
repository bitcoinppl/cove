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
import java.io.Closeable
import java.util.concurrent.atomic.AtomicBoolean

/**
 * send flow presenter - manages UI state for send flow screens
 * ported from iOS SendFlowPresenter.swift
 */
class SendFlowPresenter(
    val app: AppManager,
    val manager: WalletManager,
):  Closeable {
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
            is SendFlowAlertState.Error -> errorAlertTitle(state.error)
            is SendFlowAlertState.General -> state.title
            null -> ""
        }
    }

    private fun errorAlertTitle(error: SendFlowError): String {
        return when (error) {
            is SendFlowError.EmptyAddress,
            is SendFlowError.InvalidAddress,
            is SendFlowError.WrongNetwork,
            -> "Invalid Address"

            is SendFlowError.InvalidNumber,
            is SendFlowError.ZeroAmount,
            -> "Invalid Amount"

            is SendFlowError.InsufficientFunds,
            is SendFlowError.NoBalance,
            -> "Insufficient Funds"

            is SendFlowError.SendAmountToLow -> "Send Amount Too Low"
            is SendFlowError.UnableToGetFeeRate -> "Unable to get fee rate"
            is SendFlowError.UnableToBuildTxn -> "Unable to build transaction"
            is SendFlowError.UnableToGetMaxSend -> "Unable to get max send"
            is SendFlowError.UnableToSaveUnsignedTransaction -> "Unable to Save Unsigned Transaction"
            is SendFlowError.WalletManagerError -> "Error"
            is SendFlowError.UnableToGetFeeDetails -> "Fee Details Error"
        }
    }

    /**
     * get alert message text based on alert state
     */
    fun alertMessage(): String {
        return when (val state = alertState?.item) {
            is SendFlowAlertState.Error -> errorAlertMessage(state.error)
            is SendFlowAlertState.General -> state.message
            null -> ""
        }
    }

    private fun errorAlertMessage(error: SendFlowError): String {
        return when (error) {
            is SendFlowError.EmptyAddress ->
                "Please enter an address"

            is SendFlowError.InvalidNumber ->
                "Please enter a valid number for the amount to send"

            is SendFlowError.ZeroAmount ->
                "Can't send an empty transaction. Please enter a valid amount"

            is SendFlowError.NoBalance ->
                "You do not have any bitcoin in your wallet. Please add some to send a transaction"

            is SendFlowError.InvalidAddress ->
                "The address ${error.address} is invalid"

            is SendFlowError.WrongNetwork ->
                "The address ${error.address} is on the wrong network, it is for ${error.validFor}. You are on ${error.current}"

            is SendFlowError.InsufficientFunds ->
                "You do not have enough bitcoin in your wallet to cover the amount plus fees"

            is SendFlowError.SendAmountToLow ->
                "Send amount is too low. Please send at least 5000 sats"

            is SendFlowError.UnableToGetFeeRate ->
                "Are you connected to the internet?"

            is SendFlowError.WalletManagerError ->
                error.msg.describe

            is SendFlowError.UnableToGetFeeDetails ->
                error.msg

            is SendFlowError.UnableToBuildTxn ->
                error.msg

            is SendFlowError.UnableToGetMaxSend ->
                error.msg

            is SendFlowError.UnableToSaveUnsignedTransaction ->
                error.msg
        }
    }

    /**
     * get alert button action based on error type
     */
    fun alertButtonAction(): (() -> Unit)? {
        return when (val state = alertState?.item) {
            is SendFlowAlertState.Error -> errorAlertButtonAction(state.error)
            is SendFlowAlertState.General -> {
                { alertState = null }
            }
            null -> null
        }
    }

    private fun errorAlertButtonAction(error: SendFlowError): () -> Unit {
        return when (error) {
            is SendFlowError.EmptyAddress,
            is SendFlowError.WrongNetwork,
            is SendFlowError.InvalidAddress,
            -> {
                {
                    alertState = null
                    focusField = SetAmountFocusField.ADDRESS
                }
            }

            is SendFlowError.NoBalance -> {
                {
                    alertState = null
                    app.popRoute()
                }
            }

            is SendFlowError.InvalidNumber,
            is SendFlowError.InsufficientFunds,
            is SendFlowError.SendAmountToLow,
            is SendFlowError.ZeroAmount,
            is SendFlowError.WalletManagerError,
            is SendFlowError.UnableToGetFeeDetails,
            is SendFlowError.UnableToGetFeeRate,
            is SendFlowError.UnableToBuildTxn,
            is SendFlowError.UnableToSaveUnsignedTransaction,
            is SendFlowError.UnableToGetMaxSend,
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

/**
 * focus fields for send amount screen
 */
enum class SetAmountFocusField {
    ADDRESS,
    AMOUNT,
}
