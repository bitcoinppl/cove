package org.bitcoinppl.cove

import androidx.compose.runtime.Stable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.launch
import org.bitcoinppl.cove_core.*
import org.bitcoinppl.cove_core.types.*
import java.time.Instant
import java.util.concurrent.atomic.AtomicBoolean

enum class UnlockMode {
    MAIN,
    DECOY,
    WIPE,
    LOCKED,
}

/**
 * auth manager - manages authentication state
 * ported from iOS AuthManager.swift
 */
@Stable
class AuthManager private constructor() : AuthManagerReconciler {
    private val tag = "AuthManager"

    private val mainScope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)

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

    var isWipeDataPinEnabled by mutableStateOf<Boolean>(rust.isWipeDataPinEnabled())
        private set

    var isDecoyPinEnabled by mutableStateOf<Boolean>(rust.isDecoyPinEnabled())
        private set

    val isAuthEnabled: Boolean
        get() = type != AuthType.NONE

    val lockedAt: Instant?
        get() =
            withRustOr(null, "lockedAt") {
                lockedAt()
            }?.let { Instant.ofEpochSecond(it.toLong()) }

    init {
        logDebug("Initializing AuthManager")
        rust.listenForUpdates(this)
    }

    private fun <T> withRust(
        callName: String,
        block: RustAuthManager.() -> T,
    ): T = rustGuard.withHandle(rust, callName, block)

    private fun <T> withRustOr(
        defaultValue: T,
        callName: String,
        block: RustAuthManager.() -> T,
    ): T = rustGuard.withHandleOr(rust, defaultValue, callName, block)

    companion object {
        @Volatile
        private var instance: AuthManager? = null

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
                    AuthManager().also { instance = it }
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
            withRust("setLockedAt") {
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
            withRust("setLockedAt") {
                setLockedAt(lockedAt = 0UL)
            }
        } catch (e: Exception) {
            android.util.Log.e(tag, "failed to unlock", e)
        }
    }

    /**
     * check if in decoy mode
     */
    fun isInDecoyMode(): Boolean =
        withRustOr(false, "isInDecoyMode") {
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
        withRustOr(false, "checkDecoyPin") {
            checkDecoyPin(pin)
        }

    /**
     * check if PIN is wipe data PIN
     */
    fun checkWipeDataPin(pin: String): Boolean =
        withRustOr(false, "checkWipeDataPin") {
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
    fun handleAndReturnUnlockMode(pin: String): UnlockMode {
        // check if PIN matches main wallet PIN
        if (AuthPin().use { it.check(pin) }) {
            if (Database().globalConfig().isInDecoyMode()) {
                switchToMainMode()
            }
            unlock()
            return UnlockMode.MAIN
        }

        // check if the entered pin is the decoy pin, if so enter decoy mode
        if (checkDecoyPin(pin)) {
            // enter decoy mode if not already in decoy mode and reset app and router
            if (Database().globalConfig().isInMainMode()) {
                try {
                    withRust("switchToDecoyMode") {
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

        // check if the entered pin is a wipeDataPin
        // if so wipe the data
        if (checkWipeDataPin(pin)) {
            App.isLoading = true
            try {
                App.dangerousWipeAllData()

                // reset auth manager
                val oldRust = rust
                rust = RustAuthManager()
                rustGuard.markOpen()
                rust.listenForUpdates(this)
                oldRust.close()
                unlock()

                type = AuthType.NONE

                // reset app manager
                App.reset()

                return UnlockMode.WIPE
            } catch (e: Exception) {
                android.util.Log.e(tag, "failed to wipe all data", e)
                return UnlockMode.LOCKED
            }
        }

        return UnlockMode.LOCKED
    }

    /**
     * switch to main mode from decoy mode
     */
    fun switchToMainMode() {
        try {
            withRust("switchToMainMode") {
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
                        withRustOr(isWipeDataPinEnabled, "isWipeDataPinEnabled") {
                            isWipeDataPinEnabled()
                        }
                }

                is AuthManagerReconcileMessage.DecoyPinChanged -> {
                    isDecoyPinEnabled =
                        withRustOr(isDecoyPinEnabled, "isDecoyPinEnabled") {
                            isDecoyPinEnabled()
                        }
                }
            }
        }
    }

    fun dispatch(action: AuthManagerAction) {
        logDebug("dispatch: $action")
        withRustOr(Unit, "dispatch") {
            dispatch(action)
        }
    }

    fun validateSecurityAction(
        action: SecuritySettingsAction,
        unverifiedWalletIds: List<WalletId>,
    ): SecuritySettingsResult =
        withRust("validateSecurityAction") {
            validateSecurityAction(action, unverifiedWalletIds)
        }

    fun validateNewPin(pin: String): String? =
        withRustOr(null, "validateNewPin") {
            validateNewPin(pin)
        }

    fun setWipeDataPin(pin: String) {
        withRust("setWipeDataPin") {
            setWipeDataPin(pin)
        }
    }

    fun setDecoyPin(pin: String) {
        withRust("setDecoyPin") {
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
