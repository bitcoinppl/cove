package org.bitcoinppl.cove

import androidx.compose.runtime.Stable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.launch
import org.bitcoinppl.cove_core.FeeBumpFeeWarning
import org.bitcoinppl.cove_core.FeeBumpManagerAction
import org.bitcoinppl.cove_core.FeeBumpManagerReconcileMessage
import org.bitcoinppl.cove_core.FeeBumpManagerReconciler
import org.bitcoinppl.cove_core.FeeBumpOptions
import org.bitcoinppl.cove_core.FeeBumpRateException
import org.bitcoinppl.cove_core.FeeBumpStatus
import org.bitcoinppl.cove_core.Route
import org.bitcoinppl.cove_core.RustFeeBumpManager
import org.bitcoinppl.cove_core.types.FeeRate
import org.bitcoinppl.cove_core.types.FeeRateOptionWithTotalFee
import org.bitcoinppl.cove_core.types.TxId
import java.io.Closeable
import java.util.concurrent.atomic.AtomicBoolean

/**
 * Speeds up one pending send by replacing it with a transaction that pays a higher fee
 *
 * The sheet or popup that opens it owns it and closes it when it leaves composition. It is never
 * cached: Rust loads the options when it is made, so every new screen gets fresh rates
 */
@Stable
class FeeBumpManager internal constructor(
    private val rust: RustFeeBumpManager,
    val txId: TxId,
) : FeeBumpManagerReconciler,
    Closeable {
    private val tag = "FeeBumpManager"

    private val mainScope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)
    private val isClosed = AtomicBoolean(false)
    private val rustGuard =
        RustHandleGuard(
            ownerName = "FeeBumpManager",
            handleName = "RustFeeBumpManager",
            isClosed = isClosed,
        ) {
            Log.w(tag, it)
        }

    var status by mutableStateOf<FeeBumpStatus>(FeeBumpStatus.Loading)
        private set

    var selected by mutableStateOf<FeeRateOptionWithTotalFee?>(null)
        private set

    /** Set from Continue until Rust hands over the next screen or says why it can't */
    var isContinuing by mutableStateOf(false)
        private set

    /** The confirm or export route Continue built, which the screen that owns this manager pushes */
    var readyRoute by mutableStateOf<Route?>(null)
        private set

    var alert by mutableStateOf<TaggedItem<FeeBumpAlert>?>(null)

    // counts selections, so a custom rate that finishes pricing late can't replace a later choice;
    // only touched on the main thread
    private var selectionCount = 0

    init {
        status = rust.status()
        selected = rust.selectedFeeRate()
        rust.listenForUpdates(this)
    }

    /** The options while they are ready, null while loading or when the send can't be sped up */
    val options: FeeBumpOptions?
        get() = (status as? FeeBumpStatus.Ready)?.v1

    val unavailableMessage: String?
        get() = (status as? FeeBumpStatus.Unavailable)?.v1?.userMessage()

    /** The replacement's total fee at a custom rate, failing below the lowest rate a node takes */
    suspend fun customFeeOption(feeRate: FeeRate): FeeRateOptionWithTotalFee =
        rustGuard.withHandleSuspend(rust) {
            customFeeOption(feeRate)
        }

    fun select(option: FeeRateOptionWithTotalFee) {
        selectionCount += 1
        selected = option
        dispatch(FeeBumpManagerAction.SelectFeeRate(option))
    }

    /**
     * Prices a custom rate and selects it once priced, unless the user picked another fee meanwhile
     *
     * For a custom rate sheet that closed before its typed rate finished pricing: the work runs
     * here, as this manager outlives that sheet
     */
    fun selectCustomFeeRate(feeRate: FeeRate) {
        val countAtStart = selectionCount

        mainScope.launch {
            runCatchingCancellable(tag, "Unable to select a fee bump at ${feeRate.satPerVb()} sat/vB") {
                customFeeOption(feeRate)
            }.onSuccess { option ->
                // the user picked another fee while this rate was pricing
                if (selectionCount != countAtStart) return@onSuccess

                select(option)
            }.onFailure { error ->
                // the sheet that would have shown why closed already, so say it here
                if (selectionCount != countAtStart) return@onFailure
                val message = customRateFailureMessage(error) ?: return@onFailure
                alert = TaggedItem(FeeBumpAlert.CustomRateFailed(message))
            }
        }
    }

    /** Builds the replacement and moves on to confirm or export it */
    fun continueWithSelected() {
        if (selected == null || options == null || isContinuing) return

        isContinuing = true
        dispatch(FeeBumpManagerAction.Continue)
    }

    fun acknowledge(warning: FeeBumpFeeWarning) {
        isContinuing = true
        dispatch(FeeBumpManagerAction.AcknowledgeFeeWarning(warning))
    }

    fun screenAppeared() {
        isContinuing = false
        dispatch(FeeBumpManagerAction.ScreenAppeared)
    }

    private fun apply(message: FeeBumpManagerReconcileMessage) {
        when (message) {
            is FeeBumpManagerReconcileMessage.UpdateStatus -> {
                status = message.v1

                // a send that confirmed or was replaced meanwhile can't continue any more
                if (message.v1 is FeeBumpStatus.Unavailable) isContinuing = false
            }

            is FeeBumpManagerReconcileMessage.UpdateSelectedFeeRate -> selected = message.v1

            is FeeBumpManagerReconcileMessage.ContinueFailed -> {
                isContinuing = false
                alert = TaggedItem(FeeBumpAlert.ContinueFailed(message.v1))
            }

            is FeeBumpManagerReconcileMessage.FeeWarning -> {
                isContinuing = false
                alert = TaggedItem(FeeBumpAlert.FeeWarning(message.v1))
            }

            // stays continuing while the screen closes, so Continue can't be tapped again
            is FeeBumpManagerReconcileMessage.ReadyToConfirm -> readyRoute = message.v1
        }
    }

    override fun reconcile(message: FeeBumpManagerReconcileMessage) {
        if (isClosed.get()) return

        Log.d(tag, "reconcile: ${message::class.simpleName}")
        mainScope.launch { apply(message) }
    }

    override fun reconcileMany(messages: List<FeeBumpManagerReconcileMessage>) {
        if (isClosed.get()) return

        Log.d(tag, "reconcile_messages: ${messages.size} messages")
        mainScope.launch { messages.forEach(::apply) }
    }

    // matches the other managers, which run Rust dispatch on IO without an injected dispatcher
    @Suppress("InjectDispatcher")
    fun dispatch(action: FeeBumpManagerAction) {
        Log.d(tag, "dispatch: ${action::class.simpleName}")
        mainScope.launch(Dispatchers.IO) {
            rustGuard.withHandleOr(rust, Unit) {
                dispatch(action)
            }
        }
    }

    override fun close() {
        rustGuard.closeOnce {
            Log.d(tag, "Closing FeeBumpManager")
            mainScope.cancel()
            rust.close()
        }
    }
}

/** What the Speed Up screens tell the user in an alert */
sealed interface FeeBumpAlert {
    data class ContinueFailed(
        val message: String,
    ) : FeeBumpAlert

    data class CustomRateFailed(
        val message: String,
    ) : FeeBumpAlert

    data class FeeWarning(
        val warning: FeeBumpFeeWarning,
    ) : FeeBumpAlert
}

/** Rust's message for a custom rate that can't pay for the replacement */
internal fun customRateFailureMessage(error: Throwable): String? = (error as? FeeBumpRateException.Unavailable)?.v1
