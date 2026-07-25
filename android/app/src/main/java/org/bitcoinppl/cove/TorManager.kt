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
import org.bitcoinppl.cove_core.TorFailureOrigin
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

    /**
     * A null message means the runtime has not reported a bootstrap line yet
     */
    data class Bootstrapping(
        val percent: Int,
        val message: String?,
    ) : TorStatus

    data object Ready : TorStatus

    data object Stopped : TorStatus

    data class Failed(
        val message: String,
    ) : TorStatus
}

/**
 * Dismissible Tor alert surfaces
 */
enum class TorAlert {
    DISABLE_WARNING,
    STARTUP_FAILURE,
    ACTION_ERROR,
    CLEARNET_FALLBACK_FAILURE,
}

@Stable
class TorManager private constructor() : TorManagerReconciler {
    private val mainScope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)
    private val rust = RustTorManager()
    private val isRustClosed = AtomicBoolean(false)
    private val rustGuard =
        RustHandleGuard(
            ownerName = "TorManager",
            handleName = "RustTorManager",
            isClosed = isRustClosed,
        ) {
            Log.w(TAG, it)
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

    /**
     * The clearnet fallback offered alongside a startup failure could not be applied
     */
    var clearnetFallbackFailure by mutableStateOf<String?>(null)
        private set

    /**
     * Built-in Tor was not auto-started because repeated launches failed
     */
    var autoStartSuppressed by mutableStateOf(false)
        private set

    /**
     * A route-changing action is waiting on Rust, so the controls that start one stay disabled
     * until it lands
     */
    var isUpdatingConfig by mutableStateOf(false)
        private set

    // only mutated from the main dispatcher, so a plain counter keeps [isUpdatingConfig] true
    // while any of several overlapping actions is still in flight
    private var inFlightConfigActions = 0

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
        dispatch(TorManagerAction.Enable) { failure ->
            actionError = failure

            // enabling an already built-in config is a no-op transition in Rust and emits no
            // reconcile message, so a re-enable that the runtime accepted has to retire the
            // banner and open the startup window here
            if (failure == null && autoStartSuppressed) {
                autoStartSuppressed = false
                status = TorStatus.Bootstrapping(percent = 0, message = null)
            }
        }
    }

    /**
     * Resynchronizes published state with the runtime, whose latched status can be stale after
     * the app was suspended
     */
    fun refreshStatus() {
        rustGuard.withHandleOr(rust, Unit) { refreshStatus() }
    }

    fun disable() {
        dispatch(TorManagerAction.Disable)
    }

    fun disableConfirmed() {
        disableWarning = null
        clearnetFallbackFailure = null

        dispatch(TorManagerAction.DisableConfirmed) { failure ->
            clearnetFallbackFailure = failure
        }
    }

    fun applyConfig(config: TorConfig) {
        dispatch(TorManagerAction.SetConfig(config))
    }

    fun runConnectionTest() {
        connectionTestStateMap.clear()
        dispatch(TorManagerAction.RunConnectionTest)
    }

    fun dismiss(alert: TorAlert) {
        when (alert) {
            TorAlert.DISABLE_WARNING -> disableWarning = null
            TorAlert.STARTUP_FAILURE -> startupFailure = null
            TorAlert.ACTION_ERROR -> actionError = null
            TorAlert.CLEARNET_FALLBACK_FAILURE -> clearnetFallbackFailure = null
        }
    }

    /**
     * Runs a Tor action, handing its outcome to [onSettled] when the caller owns what happens
     * once the action lands and reporting any failure through [actionError] otherwise
     */
    @Suppress("TooGenericExceptionCaught")
    private fun dispatch(
        action: TorManagerAction,
        onSettled: ((failure: String?) -> Unit)? = null,
    ) {
        actionError = null

        // the connection test reports its own progress through [connectionTestStates]
        val changesRoute = action !is TorManagerAction.RunConnectionTest
        if (changesRoute) {
            inFlightConfigActions += 1
            isUpdatingConfig = true
        }

        mainScope.launch {
            val failure =
                try {
                    when (val result = withRustSuspend { dispatch(action) }) {
                        is TorManagerDispatchResult.Applied -> Unit
                        is TorManagerDispatchResult.DisableWarning -> disableWarning = result.v1
                    }

                    null
                } catch (error: CancellationException) {
                    throw error
                } catch (error: TorException) {
                    Log.e(TAG, "Tor action failed", error)
                    error.toString()
                } catch (error: Throwable) {
                    // a Rust panic arrives as InternalException and a closed handle as
                    // IllegalStateException; neither may escape and take the process down
                    Log.e(TAG, "Tor action failed unexpectedly", error)
                    error.toString()
                } finally {
                    // a cancelled scope must not leave the controls disabled for the session
                    if (changesRoute) {
                        inFlightConfigActions -= 1
                        isUpdatingConfig = inFlightConfigActions > 0
                    }
                }

            if (onSettled != null) {
                onSettled(failure)
            } else {
                actionError = failure
            }
        }
    }

    override fun reconcile(message: TorManagerReconcileMessage) {
        mainScope.launch {
            when (message) {
                is TorManagerReconcileMessage.ConfigChanged -> {
                    config = message.v1

                    // built-in Tor blocks clearnet from the moment the config lands, so the UI
                    // has to show that startup window instead of claiming Tor is stopped
                    status =
                        when (message.v1) {
                            is TorConfig.Off -> TorStatus.Off
                            is TorConfig.BuiltIn -> TorStatus.Bootstrapping(percent = 0, message = null)
                            is TorConfig.External -> TorStatus.Stopped
                        }

                    connectionTestStateMap.clear()
                    startupFailure = null
                    autoStartSuppressed = false
                }

                is TorManagerReconcileMessage.BootstrapProgress -> {
                    status =
                        TorStatus.Bootstrapping(
                            percent = message.percent.toInt(),
                            message = message.message,
                        )
                    startupFailure = null
                    autoStartSuppressed = false
                }

                is TorManagerReconcileMessage.Ready -> {
                    status = TorStatus.Ready
                    startupFailure = null
                    autoStartSuppressed = false
                }

                // Rust only reports a plain stop while the crash-loop breaker is disengaged, so
                // both the startup alert and the auto-start banner are stale by now
                is TorManagerReconcileMessage.Stopped -> {
                    status = if (config is TorConfig.Off) TorStatus.Off else TorStatus.Stopped
                    startupFailure = null
                    autoStartSuppressed = false
                }

                is TorManagerReconcileMessage.Failed -> {
                    handleFailure(message.origin, message.error)
                }

                // the breaker is engaged here, so a startup failure notice is still accurate and
                // has to outlive this message
                is TorManagerReconcileMessage.AutoStartSuppressed -> {
                    autoStartSuppressed = true
                    status = TorStatus.Stopped
                }

                is TorManagerReconcileMessage.ConnectionTest -> {
                    connectionTestStateMap[message.v1.step] = message.v1.state
                }
            }
        }
    }

    private fun handleFailure(
        origin: TorFailureOrigin,
        error: TorException,
    ) {
        val failure = error.toString()

        // a failed connection test says nothing about the route itself, it is reported
        // per-step through connectionTestStates
        if (origin == TorFailureOrigin.CONNECTION_TEST) {
            Log.w(TAG, "Tor connection test failed: $failure")
            return
        }

        // the route is untouched by a failed clearnet swap and the dispatch that asked for it
        // reports the error itself, so surfacing it here would misattribute a healthy runtime
        if (error is TorException.ClearnetFallback) {
            Log.w(TAG, "Tor clearnet fallback failed: $failure")
            return
        }

        status = TorStatus.Failed(failure)

        if (config is TorConfig.BuiltIn) {
            startupFailure = failure
        }
    }

    companion object {
        private const val TAG = "TorManager"

        @Volatile
        private var instance: TorManager? = null

        private fun requireBootstrapComplete() {
            val step = bootstrapProgress()
            check(step == BootstrapStep.COMPLETE) {
                "TorManager initialized before bootstrap completed: $step"
            }
        }

        /**
         * The Tor manager, or null when its Rust handle cannot be created
         *
         * The Rust singleton stays poisoned once its initialization panics, so every retry
         * fails the same way and callers have to render without Tor rather than crash
         */
        @Suppress("TooGenericExceptionCaught")
        fun getInstanceOrNull(): TorManager? =
            instance ?: synchronized(this) {
                instance ?: try {
                    requireBootstrapComplete()
                    TorManager().also { instance = it }
                } catch (error: Throwable) {
                    Log.e(TAG, "Tor is unavailable, its manager could not be created", error)
                    null
                }
            }
    }
}
