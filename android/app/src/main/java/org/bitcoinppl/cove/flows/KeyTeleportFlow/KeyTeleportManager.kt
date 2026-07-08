package org.bitcoinppl.cove.flows.KeyTeleportFlow

import androidx.compose.runtime.Stable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.launch
import org.bitcoinppl.cove.Log
import org.bitcoinppl.cove.RustHandleGuard
import org.bitcoinppl.cove_core.KeyTeleportAlert
import org.bitcoinppl.cove_core.KeyTeleportManagerAction
import org.bitcoinppl.cove_core.KeyTeleportManagerReconcileMessage
import org.bitcoinppl.cove_core.KeyTeleportManagerReconciler
import org.bitcoinppl.cove_core.KeyTeleportManagerState
import org.bitcoinppl.cove_core.RustKeyTeleportManager
import org.bitcoinppl.cove_core.StringOrData
import org.bitcoinppl.cove_core.types.WalletId
import java.io.Closeable
import java.util.concurrent.atomic.AtomicBoolean

@Stable
class KeyTeleportManager internal constructor(
    private val rust: RustKeyTeleportManager,
) : KeyTeleportManagerReconciler,
    Closeable {
    private val tag = "KeyTeleportManager"
    private val mainScope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)
    private val isClosed = AtomicBoolean(false)
    private val rustGuard =
        RustHandleGuard(
            ownerName = "KeyTeleportManager",
            handleName = "RustKeyTeleportManager",
            isClosed = isClosed,
        ) {
            Log.w(tag, it)
        }

    var state by mutableStateOf(rust.state())
        private set

    var alert by mutableStateOf<KeyTeleportAlert?>(null)
        private set

    init {
        rust.listenForUpdates(this)
    }

    fun dispatch(action: KeyTeleportManagerAction) {
        if (isClosed.get()) return

        mainScope.launch(Dispatchers.Default) {
            runCatching {
                rustGuard.withHandle(rust) {
                    dispatch(action)
                }
            }.onFailure {
                Log.e(tag, "Unable to dispatch Key Teleport action", it)
            }
        }
    }

    fun ingest(input: StringOrData) {
        dispatch(KeyTeleportManagerAction.Ingest(input))
    }

    fun clearAlertForDisplay() {
        alert = null
    }

    fun revealMnemonicWords(): List<String> =
        rustGuard.withHandleOr(rust, emptyList()) {
            revealMnemonicWords()
        }

    fun revealXprv(): String? =
        rustGuard.withHandleOr(rust, null) {
            revealXprv()
        }

    fun isSendEligible(walletId: WalletId): Boolean =
        rustGuard.withHandleOr(rust, false) {
            isSendEligible(walletId)
        }

    override fun reconcile(message: KeyTeleportManagerReconcileMessage) {
        mainScope.launch {
            apply(message)
        }
    }

    override fun reconcileMany(messages: List<KeyTeleportManagerReconcileMessage>) {
        mainScope.launch {
            messages.forEach { apply(it) }
        }
    }

    private fun apply(message: KeyTeleportManagerReconcileMessage) {
        when (message) {
            is KeyTeleportManagerReconcileMessage.UpdateState -> {
                state = message.v1
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
        rustGuard.closeOnce {
            runCatching {
                rust.dispatch(KeyTeleportManagerAction.Clear)
            }.onFailure {
                Log.w(tag, "Error clearing Key Teleport manager: ${it.message}")
            }
            mainScope.cancel()
            rust.close()
        }
    }
}
