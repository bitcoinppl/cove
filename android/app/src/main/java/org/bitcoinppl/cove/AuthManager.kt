package org.bitcoinppl.cove

import androidx.compose.runtime.Stable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import kotlinx.coroutines.CoroutineDispatcher
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.CoroutineStart
import kotlinx.coroutines.Deferred
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.async
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import org.bitcoinppl.cove_core.*
import org.bitcoinppl.cove_core.types.*
import java.time.Instant
import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicReference

internal class OwnerScopedCommand<T>(
    private val ownerScope: CoroutineScope,
) {
    private val active = AtomicReference<Deferred<T>?>(null)

    fun start(block: suspend () -> T): Deferred<T> {
        while (true) {
            val current = active.get()
            if (current != null && !current.isCompleted) return current
            if (current != null) {
                active.compareAndSet(current, null)
                continue
            }

            val candidate = ownerScope.async(start = CoroutineStart.LAZY) { block() }
            if (active.compareAndSet(null, candidate)) {
                candidate.invokeOnCompletion { active.compareAndSet(candidate, null) }
                candidate.start()
                return candidate
            }

            candidate.cancel()
        }
    }
}

enum class UnlockMode {
    MAIN,
    DECOY,
    WIPE,
    LOCKED,
}

sealed interface WipePresentationState {
    data object Idle : WipePresentationState
    data object Running : WipePresentationState
    data class ShutdownBlocked(val attemptId: ShutdownAttemptId) : WipePresentationState
    data class Failed(val message: String) : WipePresentationState
}

/**
 * auth manager - manages authentication state
 * ported from iOS AuthManager.swift
 */
@Stable
class AuthManager internal constructor(
    private val wipeDispatcher: CoroutineDispatcher,
) : AuthManagerReconciler {
    private val tag = "AuthManager"

    private val mainScope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)
    private val wipeCommand = OwnerScopedCommand<UnlockMode>(mainScope)

    private var rust: RustAuthManager = RustAuthManager()
        private set
    private val isRustClosed = AtomicBoolean(false)
    private val rustGuard =
        RustHandleGuard(
            ownerName = "AuthManager",
            handleName = "RustAuthManager",
            isClosed = isRustClosed,
        ) {
            android.util.Log.w(tag, it)
        }

    var type by mutableStateOf<AuthType>(Database().globalConfig().authType())
        private set

    var isLocked by mutableStateOf(true)
        private set

    var isUsingBiometrics by mutableStateOf(false)

    var mainCredentialGeneration by mutableStateOf(0L)
        private set

    var sensitiveContentGeneration by mutableStateOf(0L)
        private set

    var isWipeDataPinEnabled by mutableStateOf<Boolean>(rust.isWipeDataPinEnabled())
        private set

    var isDecoyPinEnabled by mutableStateOf<Boolean>(rust.isDecoyPinEnabled())
        private set

    var wipePresentationState by mutableStateOf<WipePresentationState>(WipePresentationState.Idle)
        private set

    val isAuthEnabled: Boolean
        get() = type != AuthType.NONE

    val lockedAt: Instant?
        get() =
            withRustOr(null) {
                lockedAt()
            }?.let { Instant.ofEpochSecond(it.toLong()) }

    init {
        logDebug("Initializing AuthManager")
        rust.listenForUpdates(this)
    }

    private fun <T> withRust(
        block: RustAuthManager.() -> T,
    ): T = rustGuard.withHandle(rust, block)

    private fun <T> withRustOr(
        defaultValue: T,
        block: RustAuthManager.() -> T,
    ): T = rustGuard.withHandleOr(rust, defaultValue, block)

    companion object {
        @Volatile
        private var instance: AuthManager? = null

        // the singleton is the production composition boundary; tests inject this dependency through the constructor
        @Suppress("InjectDispatcher")
        private val productionWipeDispatcher: CoroutineDispatcher = Dispatchers.IO

        private fun requireBootstrapComplete(owner: String) {
            val step = bootstrapProgress()
            check(step == BootstrapStep.COMPLETE) {
                "$owner initialized before bootstrap completed: $step"
            }
        }

        fun getInstance(): AuthManager =
            instance ?: synchronized(this) {
                instance ?: run {
                    requireBootstrapComplete("AuthManager")
                    AuthManager(productionWipeDispatcher).also { instance = it }
                }
            }
    }

    private fun logDebug(message: String) {
        android.util.Log.d(tag, message)
    }

    /**
     * lock the app - requires authentication to unlock
     */
    fun lock() {
        if (!isAuthEnabled) return
        val now = (System.currentTimeMillis() / 1000).toULong()
        logDebug("[AUTH] locking at $now")
        isLocked = true
        try {
            withRust {
                setLockedAt(lockedAt = now)
            }
        } catch (e: Exception) {
            android.util.Log.e(tag, "failed to set locked at", e)
        }
    }

    /**
     * unlock the app
     */
    fun unlock() {
        isLocked = false
        try {
            withRust {
                setLockedAt(lockedAt = 0UL)
            }
        } catch (e: Exception) {
            android.util.Log.e(tag, "failed to unlock", e)
        }
    }

    internal fun completeMainBiometricAuthentication() {
        if (isInDecoyMode()) {
            switchToMainMode()
        }

        recordMainCredentialAuthentication()
        unlock()
    }

    internal fun concealSensitiveContent() {
        sensitiveContentGeneration += 1
    }

    /**
     * check if in decoy mode
     */
    fun isInDecoyMode(): Boolean =
        withRustOr(false) {
            isInDecoyMode()
        }

    /**
     * check if PIN matches main wallet PIN
     */
    fun checkPin(pin: String): Boolean = AuthPin().use { it.check(pin) }

    /**
     * check if PIN is decoy PIN
     */
    fun checkDecoyPin(pin: String): Boolean =
        withRustOr(false) {
            checkDecoyPin(pin)
        }

    /**
     * check if PIN is wipe data PIN
     */
    fun checkWipeDataPin(pin: String): Boolean =
        withRustOr(false) {
            checkWipeDataPin(pin)
        }

    /**
     * reset app and select wallet (helper to avoid duplication)
     */
    private fun resetAppAndSelectWallet() {
        val app = App
        app.reset()
        app.isLoading = true

        // select the latest (most recently used) wallet or navigate to new wallet flow
        app.trySelectLatestOrNewWallet()
    }

    /**
     * handle PIN entry and return unlock mode
     * this is the main entry point for authentication
     */
    suspend fun handleAndReturnUnlockMode(pin: String): UnlockMode =
        when {
            checkPin(pin) -> unlockWithMainPin()
            checkDecoyPin(pin) -> unlockWithDecoyPin()
            checkWipeDataPin(pin) -> {
                startWipe(null).await()
            }
            else -> UnlockMode.LOCKED
        }

    private fun unlockWithMainPin(): UnlockMode {
        if (Database().globalConfig().isInDecoyMode()) {
            switchToMainMode()
        }

        recordMainCredentialAuthentication()
        unlock()
        return UnlockMode.MAIN
    }

    private fun unlockWithDecoyPin(): UnlockMode {
        // enter decoy mode if not already in decoy mode and reset app and router
        if (Database().globalConfig().isInMainMode()) {
            try {
                withRust {
                    switchToDecoyMode()
                }
                resetAppAndSelectWallet()
            } catch (e: Exception) {
                android.util.Log.e(tag, "failed to switch to decoy mode", e)
                return UnlockMode.LOCKED
            }
        }

        unlock()
        return UnlockMode.DECOY
    }

    fun retryWipe(attemptId: ShutdownAttemptId) {
        if (wipePresentationState == WipePresentationState.Running) return

        startWipe(attemptId)
    }

    fun cancelWipe(attemptId: ShutdownAttemptId) {
        App.cancelDangerousWipe(attemptId)
        wipePresentationState = WipePresentationState.Idle
    }

    fun clearWipeFailure() {
        wipePresentationState = WipePresentationState.Idle
    }

    private fun startWipe(attemptId: ShutdownAttemptId?): Deferred<UnlockMode> =
        wipeCommand.start {
            wipePresentationState = WipePresentationState.Running
            finishWipe(attemptId)
        }

    private suspend fun finishWipe(attemptId: ShutdownAttemptId?): UnlockMode {
        val result =
            runCatching {
                withContext(wipeDispatcher) {
                    if (attemptId == null) {
                        App.dangerousWipeAllData()
                    } else {
                        App.retryDangerousWipeAllData(attemptId)
                    }
                }
            }

        result.exceptionOrNull()?.let { error ->
            val lifecycle = (error as? AppException.WalletLifecycle)?.v1
            if (lifecycle is WalletLifecycleFailure.ShutdownBlocked) {
                wipePresentationState = WipePresentationState.ShutdownBlocked(lifecycle.attemptId)
            } else {
                android.util.Log.e(tag, "failed to wipe all data", error)
                wipePresentationState =
                    WipePresentationState.Failed(error.message ?: "Unable to remove local data")
            }

            return UnlockMode.LOCKED
        }

        val oldRust = rust
        rust = RustAuthManager()
        rustGuard.markOpen()
        rust.listenForUpdates(this)
        oldRust.close()
        unlock()
        type = AuthType.NONE
        wipePresentationState = WipePresentationState.Idle
        App.reset()

        return UnlockMode.WIPE
    }

    private fun recordMainCredentialAuthentication() {
        mainCredentialGeneration += 1
    }

    /**
     * switch to main mode from decoy mode
     */
    fun switchToMainMode() {
        try {
            withRust {
                switchToMainMode()
            }
            resetAppAndSelectWallet()
        } catch (e: Exception) {
            android.util.Log.e(tag, "failed to switch to main mode", e)
        }
    }

    override fun reconcile(message: AuthManagerReconcileMessage) {
        logDebug("reconcile: $message")
        mainScope.launch {
            when (message) {
                is AuthManagerReconcileMessage.AuthTypeChanged -> {
                    type = message.v1
                }

                is AuthManagerReconcileMessage.WipeDataPinChanged -> {
                    isWipeDataPinEnabled =
                        withRustOr(isWipeDataPinEnabled) {
                            isWipeDataPinEnabled()
                        }
                }

                is AuthManagerReconcileMessage.DecoyPinChanged -> {
                    isDecoyPinEnabled =
                        withRustOr(isDecoyPinEnabled) {
                            isDecoyPinEnabled()
                        }
                }
            }
        }
    }

    fun dispatch(action: AuthManagerAction) {
        logDebug("dispatch: $action")
        withRustOr(Unit) {
            dispatch(action)
        }
    }

    fun validateSecurityAction(
        action: SecuritySettingsAction,
        unverifiedWalletIds: List<WalletId>,
    ): SecuritySettingsResult =
        withRust {
            validateSecurityAction(action, unverifiedWalletIds)
        }

    fun validateNewPin(pin: String): String? =
        withRustOr(null) {
            validateNewPin(pin)
        }

    fun setWipeDataPin(pin: String) {
        withRust {
            setWipeDataPin(pin)
        }
    }

    fun setDecoyPin(pin: String) {
        withRust {
            setDecoyPin(pin)
        }
    }

    fun closeRust() {
        rustGuard.closeOnce {
            rust.close()
        }
    }
}

// global accessor for convenience
val Auth: AuthManager
    get() = AuthManager.getInstance()
