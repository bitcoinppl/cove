package org.bitcoinppl.cove

import androidx.compose.runtime.Stable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateMapOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.launch
import org.bitcoinppl.cove_core.BootstrapStep
import org.bitcoinppl.cove_core.RustTorManager
import org.bitcoinppl.cove_core.TorConfig
import org.bitcoinppl.cove_core.TorDisableWarning
import org.bitcoinppl.cove_core.TorException
import org.bitcoinppl.cove_core.TorManagerAction
import org.bitcoinppl.cove_core.TorManagerDispatchResult
import org.bitcoinppl.cove_core.TorManagerReconcileMessage
import org.bitcoinppl.cove_core.TorManagerReconciler
import org.bitcoinppl.cove_core.TorTestState
import org.bitcoinppl.cove_core.TorTestStep
import org.bitcoinppl.cove_core.bootstrapProgress
import java.util.concurrent.atomic.AtomicBoolean

sealed interface TorStatus {
    data object Off : TorStatus

    data class Bootstrapping(
        val percent: Int,
        val message: String,
    ) : TorStatus

    data object Ready : TorStatus

    data object Stopped : TorStatus

    data class Failed(
        val message: String,
    ) : TorStatus
}

@Stable
class TorManager private constructor() : TorManagerReconciler {
    private val tag = "TorManager"
    private val mainScope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)
    private val rust = RustTorManager()
    private val isRustClosed = AtomicBoolean(false)
    private val rustGuard =
        RustHandleGuard(
            ownerName = "TorManager",
            handleName = "RustTorManager",
            isClosed = isRustClosed,
        ) {
            Log.w(tag, it)
        }

    var config by mutableStateOf<TorConfig>(TorConfig.Off)
        private set

    var status by mutableStateOf<TorStatus>(TorStatus.Off)
        private set

    private val connectionTestStateMap = mutableStateMapOf<TorTestStep, TorTestState>()

    val connectionTestStates: Map<TorTestStep, TorTestState>
        get() = connectionTestStateMap

    var disableWarning by mutableStateOf<TorDisableWarning?>(null)
        private set

    var startupFailure by mutableStateOf<String?>(null)
        private set

    var actionError by mutableStateOf<String?>(null)
        private set

    val isEnabled: Boolean
        get() = config !is TorConfig.Off

    val isConnectionTestRunning: Boolean
        get() = connectionTestStateMap.values.any { it is TorTestState.Running }

    init {
        rust.listenForUpdates(this)
    }

    private suspend fun withRustSuspend(
        block: suspend RustTorManager.() -> TorManagerDispatchResult,
    ): TorManagerDispatchResult = rustGuard.withHandleSuspend(rust, block)

    fun enable() {
        dispatch(TorManagerAction.Enable)
    }

    fun disable() {
        dispatch(TorManagerAction.Disable)
    }

    fun disableConfirmed() {
        disableWarning = null
        dispatch(TorManagerAction.DisableConfirmed)
    }

    fun applyConfig(config: TorConfig) {
        dispatch(TorManagerAction.SetConfig(config))
    }

    fun runConnectionTest() {
        connectionTestStateMap.clear()
        dispatch(TorManagerAction.RunConnectionTest)
    }

    fun dismissDisableWarning() {
        disableWarning = null
    }

    fun dismissStartupFailure() {
        startupFailure = null
    }

    fun dismissActionError() {
        actionError = null
    }

    private fun dispatch(action: TorManagerAction) {
        actionError = null
        mainScope.launch {
            try {
                when (val result = withRustSuspend { dispatch(action) }) {
                    is TorManagerDispatchResult.Applied -> Unit
                    is TorManagerDispatchResult.DisableWarning -> disableWarning = result.v1
                }
            } catch (error: CancellationException) {
                throw error
            } catch (error: TorException) {
                actionError = error.toString()
                Log.e(tag, "Tor action failed", error)
            }
        }
    }

    override fun reconcile(message: TorManagerReconcileMessage) {
        mainScope.launch {
            when (message) {
                is TorManagerReconcileMessage.ConfigChanged -> {
                    config = message.v1
                    status = if (message.v1 is TorConfig.Off) TorStatus.Off else TorStatus.Stopped
                    connectionTestStateMap.clear()
                    startupFailure = null
                }

                is TorManagerReconcileMessage.BootstrapProgress -> {
                    status =
                        TorStatus.Bootstrapping(
                            percent = message.percent.toInt(),
                            message = message.message,
                        )
                    startupFailure = null
                }

                is TorManagerReconcileMessage.Ready -> {
                    status = TorStatus.Ready
                    startupFailure = null
                }

                is TorManagerReconcileMessage.Stopped -> {
                    status = if (config is TorConfig.Off) TorStatus.Off else TorStatus.Stopped
                    startupFailure = null
                }

                is TorManagerReconcileMessage.Failed -> {
                    val failure = message.v1.toString()
                    status = TorStatus.Failed(failure)
                    if (config is TorConfig.BuiltIn) {
                        startupFailure = failure
                    }
                }

                is TorManagerReconcileMessage.ConnectionTest -> {
                    connectionTestStateMap[message.v1.step] = message.v1.state
                }
            }
        }
    }

    companion object {
        @Volatile
        private var instance: TorManager? = null

        private fun requireBootstrapComplete() {
            val step = bootstrapProgress()
            check(step == BootstrapStep.COMPLETE) {
                "TorManager initialized before bootstrap completed: $step"
            }
        }

        fun getInstance(): TorManager =
            instance ?: synchronized(this) {
                instance ?: run {
                    requireBootstrapComplete()
                    TorManager().also { instance = it }
                }
            }
    }
}
