package org.bitcoinppl.cove.flows.keyteleport

import androidx.compose.runtime.Stable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.CoroutineDispatcher
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import org.bitcoinppl.cove.Log
import org.bitcoinppl.cove.RustHandleGuard
import org.bitcoinppl.cove_core.InternalException
import org.bitcoinppl.cove_core.KeyTeleportAlert
import org.bitcoinppl.cove_core.KeyTeleportManagerAction
import org.bitcoinppl.cove_core.KeyTeleportManagerReconcileMessage
import org.bitcoinppl.cove_core.KeyTeleportManagerReconciler
import org.bitcoinppl.cove_core.KeyTeleportManagerState
import org.bitcoinppl.cove_core.RustKeyTeleportManager
import org.bitcoinppl.cove_core.types.WalletId
import java.io.Closeable
import java.util.concurrent.atomic.AtomicBoolean

internal enum class KeyTeleportFlowDirection {
    RECEIVE,
    SEND,
}

internal enum class KeyTeleportSendStartResult {
    ACCEPTED,
    REJECTED,
}

@Stable
class KeyTeleportManager internal constructor(
    private val rust: RustKeyTeleportManager,
    private val rustDispatcher: CoroutineDispatcher = Dispatchers.Default.limitedParallelism(1),
) : KeyTeleportManagerReconciler,
    Closeable {
    private val tag = "KeyTeleportManager"
    private val mainScope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)
    private val rustScope = CoroutineScope(SupervisorJob() + rustDispatcher)
    private val acceptingActions = AtomicBoolean(true)
    private val isClosed = AtomicBoolean(false)
    private val rustGuard =
        RustHandleGuard(
            ownerName = "KeyTeleportManager",
            handleName = "RustKeyTeleportManager",
            isClosed = isClosed,
        ) {
            Log.w(tag, it)
        }

    internal val secrets = KeyTeleportSecrets(rust, rustGuard)

    var state by mutableStateOf(rust.state())
        private set

    var alert by mutableStateOf<KeyTeleportAlert?>(null)
        private set

    var concealmentGeneration by mutableStateOf(0L)
        private set

    internal val activeFlowDirection: KeyTeleportFlowDirection?
        get() = state.flowDirection()

    init {
        rust.listenForUpdates(this)
    }

    fun dispatch(action: KeyTeleportManagerAction) {
        if (!acceptingActions.get()) return

        rustScope.launch {
            runCatching {
                rustGuard.withHandle(rust) {
                    dispatch(action)
                }
            }.onFailure {
                Log.e(tag, "Unable to dispatch KeyTeleport action", it)
            }
        }
    }

    private fun dispatchOwnedInput(input: org.bitcoinppl.cove_core.KeyTeleportInput) {
        if (!acceptingActions.get()) {
            input.destroy()
            return
        }

        val inputReleased = AtomicBoolean(false)
        fun releaseInput() {
            if (inputReleased.compareAndSet(false, true)) {
                input.destroy()
            }
        }

        val job =
            rustScope.launch {
                try {
                    runCatching {
                        rustGuard.withHandle(rust) {
                            dispatch(KeyTeleportManagerAction.Ingest(input))
                        }
                    }.onFailure {
                        Log.e(tag, "Unable to dispatch KeyTeleport input", it)
                    }
                } finally {
                    releaseInput()
                }
            }
        job.invokeOnCompletion {
            releaseInput()
        }
    }

    internal fun ensureReceiveStarted(): Boolean {
        return when (state.flowDirection()) {
            KeyTeleportFlowDirection.RECEIVE -> true
            KeyTeleportFlowDirection.SEND -> false
            null -> {
                dispatch(KeyTeleportManagerAction.StartReceive)
                true
            }
        }
    }

    internal suspend fun startSendFromWallet(walletId: WalletId): KeyTeleportSendStartResult =
        withContext(rustDispatcher) {
            if (!acceptingActions.get()) return@withContext KeyTeleportSendStartResult.REJECTED

            try {
                val acceptedState: KeyTeleportManagerState? =
                    rustGuard.withHandle(rust) {
                        val currentState = state()
                        val canStart =
                            try {
                                currentState.canStartSendFromWallet()
                            } finally {
                                currentState.destroy()
                            }
                        if (!canStart) return@withHandle null

                        dispatch(KeyTeleportManagerAction.StartSendFromWallet(walletId))
                        state()
                    }
                if (acceptedState == null) {
                    return@withContext KeyTeleportSendStartResult.REJECTED
                }

                try {
                    if (acceptingActions.get()) {
                        acceptedState.sendStartResult()
                    } else {
                        KeyTeleportSendStartResult.REJECTED
                    }
                } finally {
                    acceptedState.destroy()
                }
            } catch (e: CancellationException) {
                throw e
            } catch (e: InternalException) {
                Log.e(tag, "Unable to start KeyTeleport send", e)
                KeyTeleportSendStartResult.REJECTED
            } catch (e: IllegalStateException) {
                Log.e(tag, "Unable to start KeyTeleport send", e)
                KeyTeleportSendStartResult.REJECTED
            }
        }

    internal fun ingest(
        input: org.bitcoinppl.cove_core.KeyTeleportInput,
        direction: KeyTeleportFlowDirection,
    ): Boolean {
        val activeDirection = state.flowDirection()
        if (activeDirection != null && activeDirection != direction) {
            input.destroy()
            return false
        }

        dispatchOwnedInput(input)

        return true
    }

    internal fun concealSensitiveContent() {
        if (!acceptingActions.get()) return

        concealmentGeneration += 1
        dispatch(KeyTeleportManagerAction.HideXprv)
    }

    fun clearAlertForDisplay() {
        alert = null
    }

    override fun reconcile(message: KeyTeleportManagerReconcileMessage) {
        if (!acceptingActions.get()) {
            message.destroy()
            return
        }

        val applied = AtomicBoolean(false)
        val job = mainScope.launch {
            if (acceptingActions.get()) {
                apply(message)
                applied.set(true)
            } else {
                message.destroy()
                applied.set(true)
            }
        }
        job.invokeOnCompletion {
            if (!applied.get()) {
                message.destroy()
            }
        }
    }

    override fun reconcileMany(messages: List<KeyTeleportManagerReconcileMessage>) {
        messages.forEach(::reconcile)
    }

    private fun apply(message: KeyTeleportManagerReconcileMessage) {
        when (message) {
            is KeyTeleportManagerReconcileMessage.UpdateState -> {
                val previousState = state
                state = message.v1
                previousState.destroy()
            }

            is KeyTeleportManagerReconcileMessage.SetAlert -> {
                alert = message.v1
            }

            is KeyTeleportManagerReconcileMessage.ClearAlert -> {
                alert = null
            }
        }
    }

    override fun close() {
        if (!acceptingActions.compareAndSet(true, false)) return

        concealmentGeneration += 1
        val previousState = state
        state = KeyTeleportManagerState.Idle
        previousState.destroy()
        mainScope.cancel()
        rustScope.launch {
            rustGuard.closeOnce {
                runCatching {
                    rust.dispatch(KeyTeleportManagerAction.Clear)
                }.onFailure {
                    Log.w(tag, "Error clearing KeyTeleport manager: ${it.message}")
                }
                rust.close()
            }
            rustScope.cancel()
        }
    }
}

internal class KeyTeleportSecrets(
    private val rust: RustKeyTeleportManager,
    private val rustGuard: RustHandleGuard,
) {
    fun revealMnemonicWords(): List<String>? =
        rustGuard.withHandleOr(rust, null) {
            revealMnemonicWords()
        }

    fun revealXprv(): String? =
        rustGuard.withHandleOr(rust, null) {
            revealXprv()
        }
}

internal fun KeyTeleportManagerState.flowDirection(): KeyTeleportFlowDirection? =
    when (this) {
        is KeyTeleportManagerState.ReceiveReady,
        is KeyTeleportManagerState.ReceiveError,
        is KeyTeleportManagerState.ReceiveEnterPassword,
        is KeyTeleportManagerState.ReceiveMnemonicReview,
        is KeyTeleportManagerState.ReceiveXprvReview,
        is KeyTeleportManagerState.ReceiveMessageReview,
        is KeyTeleportManagerState.ReceiveImportedWallet,
        is KeyTeleportManagerState.ReceiveAlreadyImportedWallet,
        -> KeyTeleportFlowDirection.RECEIVE

        is KeyTeleportManagerState.SendAwaitReceiver,
        is KeyTeleportManagerState.SendChooseWallet,
        is KeyTeleportManagerState.SendEnterCode,
        is KeyTeleportManagerState.SendReady,
        -> KeyTeleportFlowDirection.SEND

        is KeyTeleportManagerState.Idle -> null
    }

internal fun KeyTeleportManagerState.canStartSendFromWallet(): Boolean =
    this is KeyTeleportManagerState.Idle

internal fun KeyTeleportManagerState.sendStartResult(): KeyTeleportSendStartResult =
    if (flowDirection() == KeyTeleportFlowDirection.SEND) {
        KeyTeleportSendStartResult.ACCEPTED
    } else {
        KeyTeleportSendStartResult.REJECTED
    }
