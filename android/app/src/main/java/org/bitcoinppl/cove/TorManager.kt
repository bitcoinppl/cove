package org.bitcoinppl.cove

import androidx.compose.runtime.Stable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.launch
import org.bitcoinppl.cove_core.RustTorManager
import org.bitcoinppl.cove_core.TorConnectionCheck
import org.bitcoinppl.cove_core.TorManagerAction
import org.bitcoinppl.cove_core.TorManagerException
import org.bitcoinppl.cove_core.TorManagerReconcileMessage
import org.bitcoinppl.cove_core.TorManagerReconciler
import org.bitcoinppl.cove_core.TorMode
import org.bitcoinppl.cove_core.TorStatus
import java.util.concurrent.atomic.AtomicBoolean

/**
 * Tor settings state for the Tor screen and the Settings row
 *
 * Wraps the process-singleton `RustTorManager`, so there is one instance for the whole process,
 * reached through [getInstance] and never closed
 */
@Stable
class TorManager internal constructor(
    private val rust: RustTorManager,
) : TorManagerReconciler {
    private val mainScope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)
    private val isClosed = AtomicBoolean(false)
    private val rustGuard =
        RustHandleGuard(
            ownerName = "TorManager",
            handleName = "RustTorManager",
            isClosed = isClosed,
        ) {
            Log.w(TAG, it)
        }

    /** The saved mode, null when the saved setting can't be read */
    var mode by mutableStateOf<TorMode?>(null)
        private set

    var status by mutableStateOf<TorStatus>(TorStatus.Off)
        private set

    /** The last refused mode change, until the screen showing it calls [consumeModeRejection] */
    var modeRejection by mutableStateOf<TaggedItem<TorManagerException>?>(null)
        private set

    init {
        rust.listenForUpdates(this)

        // listening starts before the snapshot and every message carries a full value, so an older
        // message applied after the snapshot is always followed by the one for the newer value
        val state = rust.state()
        mode = state.mode
        status = state.status
    }

    private fun <T> withRust(
        block: RustTorManager.() -> T,
    ): T = rustGuard.withHandle(rust, block)

    /** Saves and applies a mode or restarts Tor; the result arrives as a mode or a rejection */
    fun dispatch(action: TorManagerAction) {
        runCatching { withRust { dispatch(action) } }
            .onFailure { error -> Log.e(TAG, "tor action failed: $action", error) }
    }

    /** The proxy address the form starts with when no proxy is saved */
    fun defaultProxyAddress(): String = withRust { defaultProxyAddress() }

    /**
     * Returns [input] in the form the app saves
     *
     * @throws TorManagerException.InvalidProxy with a message to show under the field
     */
    fun validateProxyAddress(input: String): String = withRust { validateProxyAddress(input) }

    /**
     * Asks the Tor Project's check service, over Tor, whether traffic leaves through Tor
     *
     * @throws TorManagerException.TorOff when Tor is off
     * @throws TorManagerException.CheckFailed when the check service can't be reached
     */
    suspend fun checkConnection(): TorConnectionCheck = rustGuard.withHandleSuspend(rust) { checkConnection() }

    fun consumeModeRejection(rejection: TaggedItem<TorManagerException>) {
        if (modeRejection?.id != rejection.id) return

        modeRejection = null
    }

    override fun reconcile(message: TorManagerReconcileMessage) {
        mainScope.launch {
            apply(message)
        }
    }

    private fun apply(message: TorManagerReconcileMessage) {
        when (message) {
            is TorManagerReconcileMessage.Mode -> mode = message.v1
            is TorManagerReconcileMessage.Status -> status = message.v1
            is TorManagerReconcileMessage.ModeRejected -> modeRejection = TaggedItem(message.v1)
        }
    }

    companion object {
        private const val TAG = "TorManager"

        @Volatile
        private var instance: TorManager? = null

        fun getInstance(): TorManager =
            instance ?: synchronized(this) {
                instance ?: TorManager(RustTorManager()).also { instance = it }
            }
    }
}
