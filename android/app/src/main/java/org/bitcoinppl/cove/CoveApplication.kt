package org.bitcoinppl.cove

import android.app.Application
import android.util.Log
import org.bitcoinppl.cove_core.setRootDataDir

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
    }

    companion object {
        private const val TAG = "CoveApplication"
    }
}
