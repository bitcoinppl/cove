package org.bitcoinppl.cove.flows.SendFlow

import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import org.bitcoinppl.cove.AppManager
import org.bitcoinppl.cove.TaggedItem
import org.bitcoinppl.cove.UiText
import org.bitcoinppl.cove.WalletManager
import org.bitcoinppl.cove.localizedMessage
import org.bitcoinppl.cove.localizedTitle
import org.bitcoinppl.cove_core.*
import org.bitcoinppl.cove_core.types.*
import java.io.Closeable
import java.util.concurrent.atomic.AtomicBoolean

/** send flow presenter - manages UI state for send flow screens */
class SendFlowPresenter(
    val app: AppManager,
    val manager: WalletManager,
) : Closeable {
    // prevents alerts from reappearing during dismissal animations
    private var disappearing by mutableStateOf(false)

    private val mainScope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)
    private val isClosed = AtomicBoolean(false)

    var focusField by mutableStateOf<SetAmountFocusField?>(null)
    var sheetState by mutableStateOf<TaggedItem<SheetState>?>(null)
    var alertState by mutableStateOf<TaggedItem<SendFlowAlertState>?>(null)

    val isShowingAlert: Boolean
        get() = alertState != null && !disappearing

    var lastWorkingFeeRate by mutableStateOf<Float?>(null)
    var erroredFeeRate by mutableStateOf<Float?>(null)

    /**
     * sheet states for send flow
     */
    sealed class SheetState {
        data object Qr : SheetState()

        data object Fee : SheetState()

        data object CoinControlCustomAmount : SheetState()

        override fun equals(other: Any?): Boolean {
            if (this === other) return true
            if (other == null) return false
            return this::class == other::class
        }

        override fun hashCode(): Int = this::class.hashCode()
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
    fun alertTitle(): UiText =
        when (val state = alertState?.item) {
            is SendFlowAlertState.Error,
            is SendFlowAlertState.General,
            is SendFlowAlertState.UnableToLoadFees,
            is SendFlowAlertState.FeeTooHigh,
            is SendFlowAlertState.HighFeeWarning,
            is SendFlowAlertState.UnableToReadLockedCoins,
            is SendFlowAlertState.BalanceStillLoading,
            -> state.localizedTitle()
            null -> UiText.raw("")
        }

    /**
     * get alert message text based on alert state
     */
    fun alertMessage(): UiText =
        when (val state = alertState?.item) {
            is SendFlowAlertState.Error,
            is SendFlowAlertState.General,
            is SendFlowAlertState.UnableToLoadFees,
            is SendFlowAlertState.FeeTooHigh,
            is SendFlowAlertState.HighFeeWarning,
            is SendFlowAlertState.UnableToReadLockedCoins,
            is SendFlowAlertState.BalanceStillLoading,
            -> state.localizedMessage()
            null -> UiText.raw("")
        }

    /**
     * get alert button action based on error type
     */
    fun alertButtonAction(): (() -> Unit)? =
        when (val state = alertState?.item) {
            is SendFlowAlertState.Error -> errorAlertButtonAction(state.v1)
            is SendFlowAlertState.General,
            is SendFlowAlertState.UnableToLoadFees,
            is SendFlowAlertState.FeeTooHigh,
            is SendFlowAlertState.HighFeeWarning,
            is SendFlowAlertState.UnableToReadLockedCoins,
            is SendFlowAlertState.BalanceStillLoading,
            -> {
                { alertState = null }
            }
            null -> null
        }

    private fun errorAlertButtonAction(error: SendFlowException): () -> Unit =
        when (error) {
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

    override fun close() {
        if (!isClosed.compareAndSet(false, true)) return
        mainScope.cancel()
    }
}
