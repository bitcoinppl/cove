package org.bitcoinppl.cove

import android.app.Application
import android.util.Log
import androidx.lifecycle.DefaultLifecycleObserver
import androidx.lifecycle.LifecycleOwner
import androidx.lifecycle.ProcessLifecycleOwner
import org.bitcoinppl.cove_core.AuthType
import org.bitcoinppl.cove_core.device.Device
import org.bitcoinppl.cove_core.device.Keychain
import org.bitcoinppl.cove_core.setRootDataDir
import java.time.Instant

// auto-unlock time thresholds (in seconds) - matches iOS behavior
// TODO: make these configurable and store in database
private const val AUTO_UNLOCK_THRESHOLD_ALL_AUTH = 1L
private const val AUTO_UNLOCK_THRESHOLD_PIN_ONLY = 2L

class CoveApplication : Application() {
    override fun onCreate() {
        super.onCreate()

        // set root data directory for Android before any FFI calls
        // Android stores app data in filesDir which is app-specific
        val dataDir = filesDir.resolve(".data")

        try {
            setRootDataDir(dataDir.absolutePath)
            Log.d(TAG, "Root data directory set to: ${dataDir.absolutePath}")
        } catch (e: Exception) {
            Log.e(TAG, "Failed to set root data directory", e)
            throw RuntimeException("Failed to initialize app data directory", e)
        }

        // initialize keychain and device before any FFI calls that might use them
        try {
            Keychain(KeychainAccessor(this))
            Device(DeviceAccessor())
            Log.d(TAG, "Keychain and device initialized")
        } catch (e: Exception) {
            Log.e(TAG, "Failed to initialize keychain and device", e)
            throw RuntimeException("Failed to initialize security infrastructure", e)
        }

        // initialize app manager to ensure updater is ready before lifecycle callbacks
        AppManager.getInstance()

        // setup app lifecycle observer for auth lock/unlock
        setupLifecycleObserver()
    }

    private fun setupLifecycleObserver() {
        ProcessLifecycleOwner.get().lifecycle.addObserver(
            object : DefaultLifecycleObserver {
                override fun onStart(owner: LifecycleOwner) {
                    // app coming to foreground
                    handleForeground()
                }

                override fun onStop(owner: LifecycleOwner) {
                    // app going to background
                    handleBackground()
                }
            },
        )
    }

    private fun handleBackground() {
        try {
            val auth = Auth
            if (auth.isAuthEnabled && !auth.isLocked) {
                Log.d(TAG, "[LIFECYCLE] App going to background, locking")
                auth.lock()
            }
        } catch (e: Exception) {
            Log.e(TAG, "Error handling background", e)
        }
    }

    private fun handleForeground() {
        try {
            val auth = Auth

            // if auth is not enabled, unlock immediately
            if (!auth.isAuthEnabled) {
                Log.d(TAG, "[LIFECYCLE] Auth not enabled, unlocking")
                auth.unlock()
                return
            }

            // get time since lock
            val lockedAt = auth.lockedAt
            if (lockedAt == null) {
                Log.d(TAG, "[LIFECYCLE] No locked_at timestamp, keeping current lock state")
                return
            }

            val sinceLocked = Instant.now().epochSecond - lockedAt.epochSecond
            Log.d(TAG, "[LIFECYCLE] Time since locked: ${sinceLocked}s")

            // auto-unlock thresholds (matches iOS behavior)
            when {
                // less than 2 seconds - auto unlock only for PIN without decoy mode
                sinceLocked < AUTO_UNLOCK_THRESHOLD_PIN_ONLY &&
                    auth.type == AuthType.PIN &&
                    !auth.isDecoyPinEnabled -> {
                    Log.d(TAG, "[LIFECYCLE] < ${AUTO_UNLOCK_THRESHOLD_PIN_ONLY}s since lock (PIN only, no decoy), auto-unlocking")
                    auth.unlock()
                }
                // less than 1 second - auto unlock for all auth types
                sinceLocked < AUTO_UNLOCK_THRESHOLD_ALL_AUTH -> {
                    Log.d(TAG, "[LIFECYCLE] < ${AUTO_UNLOCK_THRESHOLD_ALL_AUTH}s since lock, auto-unlocking")
                    auth.unlock()
                }
                // otherwise - require authentication
                else -> {
                    Log.d(TAG, "[LIFECYCLE] Requiring authentication")
                }
            }
        } catch (e: Exception) {
            Log.e(TAG, "Error handling foreground", e)
        }
    }

    companion object {
        private const val TAG = "CoveApplication"
    }
}
