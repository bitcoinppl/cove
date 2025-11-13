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

    internal var rust: RustAuthManager = RustAuthManager()
        private set

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
        get() = rust.lockedAt()?.let { Instant.ofEpochSecond(it.toLong()) }

    init {
        logDebug("Initializing AuthManager")
        rust.listenForUpdates(this)
    }

    companion object {
        @Volatile
        private var instance: AuthManager? = null

        fun getInstance(): AuthManager =
            instance ?: synchronized(this) {
                instance ?: AuthManager().also { instance = it }
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
            rust.setLockedAt(lockedAt = now)
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
            rust.setLockedAt(lockedAt = 0UL)
        } catch (e: Exception) {
            android.util.Log.e(tag, "failed to unlock", e)
        }
    }

    /**
     * check if in decoy mode
     */
    fun isInDecoyMode(): Boolean = rust.isInDecoyMode()

    /**
     * check if PIN matches main wallet PIN
     */
    fun checkPin(pin: String): Boolean = AuthPin().check(pin)

    /**
     * check if PIN is decoy PIN
     */
    fun checkDecoyPin(pin: String): Boolean = rust.checkDecoyPin(pin)

    /**
     * check if PIN is wipe data PIN
     */
    fun checkWipeDataPin(pin: String): Boolean = rust.checkWipeDataPin(pin)

    /**
     * reset app and select wallet (helper to avoid duplication)
     */
    private fun resetAppAndSelectWallet() {
        val app = App
        app.reset()
        app.isLoading = true

        // select the latest (most recently used) wallet or navigate to new wallet flow
        app.rust.selectLatestOrNewWallet()
    }

    /**
     * handle PIN entry and return unlock mode
     * this is the main entry point for authentication
     */
    fun handleAndReturnUnlockMode(pin: String): UnlockMode {
        // check if PIN matches main wallet PIN
        if (AuthPin().check(pin)) {
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
                    rust.switchToDecoyMode()
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
            try {
                App.rust.dangerousWipeAllData()

                // reset auth manager
                rust = RustAuthManager()
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
            rust.switchToMainMode()
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
                    isWipeDataPinEnabled = rust.isWipeDataPinEnabled()
                }

                is AuthManagerReconcileMessage.DecoyPinChanged -> {
                    isDecoyPinEnabled = rust.isDecoyPinEnabled()
                }
            }
        }
    }

    fun dispatch(action: AuthManagerAction) {
        logDebug("dispatch: $action")
        rust.dispatch(action)
    }
}

// global accessor for convenience
val Auth: AuthManager
    get() = AuthManager.getInstance()
