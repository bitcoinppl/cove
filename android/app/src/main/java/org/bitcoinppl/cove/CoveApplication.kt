package org.bitcoinppl.cove

import android.app.Application
import android.content.ComponentCallbacks2
import android.content.res.Configuration
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

        // register memory cleanup callbacks
        // NOTE: Android does not guarantee process-level cleanup - the OS may kill the process
        // without notice. Sensitive data should be managed via Android Keystore/EncryptedSharedPreferences
        // or hardened in Rust destructors as secondary mitigation
        setupMemoryCallbacks()
    }

    override fun onLowMemory() {
        super.onLowMemory()
        Log.d(TAG, "onLowMemory called, cleaning up FFI objects")
        cleanupFfiObjects()
    }

    private fun cleanupFfiObjects() {
        try {
            // close FFI objects in reverse order of creation
            device?.close()
            device = null
            Log.d(TAG, "Device FFI object closed")
        } catch (e: Exception) {
            Log.e(TAG, "Error closing Device FFI object", e)
        }

        try {
            keychain?.close()
            keychain = null
            Log.d(TAG, "Keychain FFI object closed")
        } catch (e: Exception) {
            Log.e(TAG, "Error closing Keychain FFI object", e)
        }

        // close AppManager and AuthManager FFI objects
        try {
            AppManager.getInstance().rust.close()
            Log.d(TAG, "AppManager FFI object closed")
        } catch (e: Exception) {
            Log.e(TAG, "Error closing AppManager FFI object", e)
        }

        try {
            AuthManager.getInstance().rust.close()
            Log.d(TAG, "AuthManager FFI object closed")
        } catch (e: Exception) {
            Log.e(TAG, "Error closing AuthManager FFI object", e)
        }
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

    private fun setupMemoryCallbacks() {
        registerComponentCallbacks(
            object : ComponentCallbacks2 {
                override fun onTrimMemory(level: Int) {
                    if (level == ComponentCallbacks2.TRIM_MEMORY_COMPLETE) {
                        Log.d(TAG, "onTrimMemory(TRIM_MEMORY_COMPLETE) called, cleaning up FFI objects")
                        cleanupFfiObjects()
                    }
                }

                override fun onConfigurationChanged(newConfig: Configuration) {
                    // no-op
                }

                override fun onLowMemory() {
                    Log.d(TAG, "ComponentCallbacks2.onLowMemory called, cleaning up FFI objects")
                    cleanupFfiObjects()
                }
            },
        )
    }

    private fun handleBackground() {
        try {
            val auth = Auth

            // reset biometric flag in case it got stuck from a failed prompt
            if (auth.isUsingBiometrics) {
                auth.isUsingBiometrics = false
            }

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
