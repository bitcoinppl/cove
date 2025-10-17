package org.bitcoinppl.cove

import androidx.compose.runtime.Stable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import java.util.Date

/**
 * unlock modes for PIN authentication
 */
enum class UnlockMode {
    MAIN, DECOY, WIPE, LOCKED
}

/**
 * authentication manager (singleton)
 * handles lock state, biometric auth, and PIN validation
 * ported from iOS AuthManager.swift
 */
@Stable
class AuthManager private constructor() : AuthManagerReconciler {
    private val tag = "AuthManager"

    internal var rust: RustAuthManager = RustAuthManager()
        private set

    var type by mutableStateOf(Database().globalConfig().authType())
        private set

    var lockState by mutableStateOf(LockState.LOCKED)

    var isWipeDataPinEnabled by mutableStateOf(rust.isWipeDataPinEnabled())
        private set

    var isDecoyPinEnabled by mutableStateOf(rust.isDecoyPinEnabled())
        private set

    var isUsingBiometrics by mutableStateOf(false)

    val lockedAt: Date?
        get() = rust.lockedAt()?.let { Date(it.toLong() * 1000) }

    init {
        android.util.Log.d(tag, "Initializing AuthManager")
        rust.listenForUpdates(this)
    }

    companion object {
        @Volatile
        private var instance: AuthManager? = null

        fun getInstance(): AuthManager {
            return instance ?: synchronized(this) {
                instance ?: AuthManager().also { instance = it }
            }
        }
    }

    private fun logDebug(message: String) {
        android.util.Log.d(tag, message)
    }

    private fun logError(message: String) {
        android.util.Log.e(tag, message)
    }

    fun isInDecoyMode(): Boolean {
        return rust.isInDecoyMode()
    }

    /**
     * lock the app if auth is enabled
     */
    fun lock() {
        if (!isAuthEnabled) return

        val now = (Date().time / 1000).toULong()
        logDebug("[AUTH] locking at $now")
        lockState = LockState.LOCKED

        try {
            rust.setLockedAt(now)
        } catch (e: Exception) {
            logError("Failed to set locked at: ${e.message}")
        }
    }

    /**
     * unlock the app
     */
    fun unlock() {
        lockState = LockState.UNLOCKED
        try {
            rust.setLockedAt(0u)
        } catch (e: Exception) {
            logError("Failed to clear locked at: ${e.message}")
        }
    }

    val isAuthEnabled: Boolean
        get() = type != AuthType.NONE

    /**
     * check if the entered PIN is correct
     */
    fun checkPin(pin: String): Boolean {
        return AuthPin().check(pin)
    }

    /**
     * handle PIN entry and return the unlock mode
     * manages decoy/wipe pin logic and app resets
     */
    suspend fun handleAndReturnUnlockMode(pin: String): UnlockMode = withContext(Dispatchers.IO) {
        // check main PIN
        if (AuthPin().check(pin)) {
            // if in decoy mode, switch back to main
            if (Database().globalConfig().isInDecoyMode()) {
                switchToMainMode()
            }
            unlock()
            return@withContext UnlockMode.MAIN
        }

        // check decoy PIN
        if (checkDecoyPin(pin)) {
            // enter decoy mode if not already in decoy mode
            if (Database().globalConfig().isInMainMode()) {
                rust.switchToDecoyMode()
                unlock()

                val app = AppManager.getInstance()
                app.reset()
                app.isLoading = true

                val db = Database()
                val selectedWalletId = db.globalConfig().selectedWallet()
                if (selectedWalletId != null) {
                    try {
                        app.rust.selectWallet(selectedWalletId)
                    } catch (e: Exception) {
                        logError("Failed to select wallet: ${e.message}")
                        app.loadAndReset(RouteHelpers.newWalletSelect())
                    }
                } else {
                    app.loadAndReset(RouteHelpers.newWalletSelect())
                }
            }

            return@withContext UnlockMode.DECOY
        }

        // check wipe data PIN
        if (checkWipeDataPin(pin)) {
            AppManager.getInstance().rust.dangerousWipeAllData()

            // reset auth manager
            rust = RustAuthManager()
            unlock()
            type = AuthType.NONE

            // reset app manager
            AppManager.getInstance().reset()

            return@withContext UnlockMode.WIPE
        }

        return@withContext UnlockMode.LOCKED
    }

    /**
     * switch from decoy mode back to main mode
     */
    suspend fun switchToMainMode() = withContext(Dispatchers.IO) {
        rust.switchToMainMode()

        val app = AppManager.getInstance()
        app.reset()
        app.isLoading = true

        val db = Database()
        val selectedWalletId = db.globalConfig().selectedWallet()
        if (selectedWalletId != null) {
            try {
                app.rust.selectWallet(selectedWalletId)
            } catch (e: Exception) {
                logError("Failed to select wallet: ${e.message}")
                app.loadAndReset(RouteHelpers.newWalletSelect())
            }
        } else {
            app.loadAndReset(RouteHelpers.newWalletSelect())
        }
    }

    fun checkWipeDataPin(pin: String): Boolean {
        return rust.checkWipeDataPin(pin)
    }

    fun checkDecoyPin(pin: String): Boolean {
        return rust.checkDecoyPin(pin)
    }

    override fun reconcile(message: AuthManagerReconcileMessage) {
        logDebug("reconcile: $message")

        when (message) {
            is AuthManagerReconcileMessage.AuthTypeChanged -> {
                type = message.authType
            }

            is AuthManagerReconcileMessage.WipeDataPinChanged -> {
                isWipeDataPinEnabled = rust.isWipeDataPinEnabled()
            }

            is AuthManagerReconcileMessage.DecoyPinChanged -> {
                isDecoyPinEnabled = rust.isDecoyPinEnabled()
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
