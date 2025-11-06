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

    var isWipeDataPinEnabled by mutableStateOf<Boolean>(rust.isWipeDataPinEnabled())
        private set

    var isDecoyPinEnabled by mutableStateOf<Boolean>(rust.isDecoyPinEnabled())
        private set

    val isAuthEnabled: Boolean
        get() = type != AuthType.NONE

    init {
        logDebug("Initializing AuthManager")
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
