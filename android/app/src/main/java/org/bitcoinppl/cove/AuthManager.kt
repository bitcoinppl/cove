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
    data object Failed : WipePresentationState
}

// the wipe PIN can be entered under duress, so this copy must not reveal that a wipe ran
internal const val WIPE_FAILURE_TITLE = "Unable to Open Cove"
internal const val WIPE_FAILURE_MESSAGE = "Please try again."

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

    private val rust = RustAuthManager()
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

    var isLocked by mutableStateOf(rust.isLocked())
        private set

    var isUsingBiometrics by mutableStateOf(false)

    var mainCredentialGeneration by mutableStateOf(0L)
        private set

    /**
     * Counts app lock unlocks made with a PIN, decoy PIN, or biometric credential
     *
     * Grace-period releases and disabled authentication never increment it, so screens can tell
     * a presented credential apart from the lock simply opening
     */
    var credentialUnlockGeneration by mutableStateOf(0L)
        private set

    /**
     * Counts every time the app lock engages
     *
     * Lets a screen tell that the lock engaged even when a grace-period release has already opened
     * it again before the screen observed the lock
     */
    var lockGeneration by mutableStateOf(0L)
        private set

    var sensitiveContentGeneration by mutableStateOf(0L)
        private set

    // the lock and conceal counts when the last credential opened the app, so a credential that a
    // later lock or conceal has outlived cannot satisfy a request
    private var lastCredentialLockGeneration = 0L
    private var lastCredentialSensitiveContentGeneration = 0L

    var isWipeDataPinEnabled by mutableStateOf<Boolean>(rust.isWipeDataPinEnabled())
        private set

    var isDecoyPinEnabled by mutableStateOf<Boolean>(rust.isDecoyPinEnabled())
        private set

    var wipePresentationState by mutableStateOf<WipePresentationState>(WipePresentationState.Idle)
        private set

    val isAuthEnabled: Boolean
        get() = type != AuthType.NONE

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
     * lock the app until the user authenticates
     */
    fun lock(reason: AppLockReason) {
        logDebug("[AUTH] locking for $reason")
        lockGeneration += 1
        isLocked = withRustOr(true) { lock(reason) }
    }

    /**
     * lock the app for a send confirmation screen, the returned token releases only this lock
     */
    fun lockForSendConfirmation(): SendConfirmationLock? {
        val lock = withRustOr<SendConfirmationLock?>(null) { lockForSendConfirmation() } ?: return null
        lockGeneration += 1
        isLocked = true
        return lock
    }

    /**
     * release the lock a send confirmation screen took, other locks stay in place
     */
    fun releaseSendConfirmationLock(lock: SendConfirmationLock) {
        isLocked = withRustOr(true) { releaseSendConfirmationLock(lock) }
    }

    /**
     * skip authentication when the app comes back right after leaving the foreground
     */
    fun releaseBackgroundLockOnForeground() {
        isLocked = withRustOr(true) { releaseBackgroundLockOnForeground() }
    }

    /**
     * unlock when no authentication is set up
     */
    fun unlockIfAuthDisabled() {
        isLocked = withRustOr(true) { unlockIfAuthDisabled() }
    }

    /**
     * Lock the app with [AppLockReason.REAUTHENTICATION] before showing secret content
     *
     * Returns null when the app stays unlocked because no app lock is set up, so there is no
     * credential to ask for
     */
    fun requestCredentialUnlock(): CredentialUnlockRequest? {
        val request =
            CredentialUnlockRequest(
                generation = credentialUnlockGeneration,
                isInDecoyMode = isInDecoyMode(),
            )

        // rust decides whether the lock applies, so an unreadable auth type still asks for a credential
        lock(AppLockReason.REAUTHENTICATION)
        return request.takeIf { isLocked }
    }

    fun credentialUnlockStatus(request: CredentialUnlockRequest): CredentialUnlockStatus =
        when {
            isInDecoyMode() != request.isInDecoyMode -> CredentialUnlockStatus.SUPERSEDED
            isLocked -> CredentialUnlockStatus.PENDING
            credentialUnlockGeneration > request.generation && isLastCredentialCurrent() ->
                CredentialUnlockStatus.SATISFIED
            else -> CredentialUnlockStatus.UNLOCKED_WITHOUT_CREDENTIAL
        }

    private fun isLastCredentialCurrent(): Boolean =
        lastCredentialLockGeneration == lockGeneration &&
            lastCredentialSensitiveContentGeneration == sensitiveContentGeneration

    // only called after the user authenticated with a PIN, biometrics, or a trick PIN
    private fun unlockAuthenticated() {
        try {
            withRust {
                unlockAuthenticated()
            }

            // count the credential before publishing the unlock so observers never see the lock open without it
            lastCredentialLockGeneration = lockGeneration
            lastCredentialSensitiveContentGeneration = sensitiveContentGeneration
            credentialUnlockGeneration += 1
            isLocked = false
        } catch (e: Exception) {
            android.util.Log.e(tag, "failed to unlock", e)
        }
    }

    internal fun completeMainBiometricAuthentication() {
        if (isInDecoyMode() && !switchToMainMode()) return

        recordMainCredentialAuthentication()
        unlockAuthenticated()
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
        if (Database().globalConfig().isInDecoyMode() && !switchToMainMode()) {
            return UnlockMode.LOCKED
        }

        recordMainCredentialAuthentication()
        unlockAuthenticated()
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

        unlockAuthenticated()
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

        val wipeError = result.exceptionOrNull()
        if (wipeError != null) {
            val lifecycle = (wipeError as? AppException.WalletLifecycle)?.v1
            if (lifecycle is WalletLifecycleFailure.ShutdownBlocked) {
                wipePresentationState = WipePresentationState.ShutdownBlocked(lifecycle.attemptId)
            } else {
                android.util.Log.e(tag, "failed to wipe all data", wipeError)
                wipePresentationState = WipePresentationState.Failed
            }

            return UnlockMode.LOCKED
        }

        val completion = result.getOrThrow()
        App.applyWipeCompletion(completion)
        apply(completion.auth)

        unlockAuthenticated()
        wipePresentationState = WipePresentationState.Idle
        return UnlockMode.WIPE
    }

    private fun apply(settings: AuthSettings) {
        type = settings.authType
        isWipeDataPinEnabled = settings.isWipeDataPinEnabled
        isDecoyPinEnabled = settings.isDecoyPinEnabled
        isUsingBiometrics = false
    }

    private fun recordMainCredentialAuthentication() {
        mainCredentialGeneration += 1
    }

    /**
     * switch to main mode from decoy mode
     *
     * returns false when the switch failed so callers keep the app locked
     * instead of unlocking into the decoy projection with the main credential
     */
    fun switchToMainMode(): Boolean =
        try {
            withRust {
                switchToMainMode()
            }
            resetAppAndSelectWallet()
            true
        } catch (e: Exception) {
            android.util.Log.e(tag, "failed to switch to main mode", e)
            false
        }

    override fun reconcile(message: AuthManagerReconcileMessage) {
        logDebug("reconcile: $message")
        mainScope.launch {
            when (message) {
                is AuthManagerReconcileMessage.AuthTypeChanged -> {
                    type = message.v1
                }

                is AuthManagerReconcileMessage.WipeDataPinChanged -> {
                    isWipeDataPinEnabled = message.v1
                }

                is AuthManagerReconcileMessage.DecoyPinChanged -> {
                    isDecoyPinEnabled = message.v1
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
