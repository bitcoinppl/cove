package org.bitcoinppl.cove.testconfig

import org.bitcoinppl.cove.LifecycleConnectivityAccess
import org.bitcoinppl.cove_core.ConnectivityStatus
import org.bitcoinppl.cove_core.RustConnectivityManager
import org.bitcoinppl.cove_core.device.CloudStorageException

/**
 * Network connectivity that UI tests switch on and off
 *
 * It reports to Rust the way the production `ConnectivityMonitor` does, and the scripted cloud
 * provider fails its requests while it is off, so a test sees what a dropped network does to
 * Cloud Backup without touching the emulator's network, which the fake node and signer host
 * need. It starts connected, so tests that never call it see an always-connected device
 */
object ScriptedConnectivityAccess : LifecycleConnectivityAccess {
    @Volatile
    private var connected = true

    /** Restores the connected default for a test that relies on it */
    fun reset() {
        setConnected(true)
    }

    /**
     * Drops or restores the network
     *
     * The provider flips before Rust hears about it, as a real provider fails as soon as the
     * network goes and works again before the platform reports the reconnect
     */
    fun setConnected(value: Boolean) {
        connected = value
        pushToRust()
    }

    /** Throws the error a real cloud provider maps a missing network to */
    fun throwIfOffline() {
        if (!connected) throw CloudStorageException.Offline("scripted network is offline")
    }

    override fun isConnected(): Boolean = connected

    override fun start() {
        pushToRust()
    }

    override fun stop() = Unit

    private fun pushToRust() {
        val status =
            if (connected) {
                ConnectivityStatus.CONNECTED
            } else {
                ConnectivityStatus.DISCONNECTED
            }

        RustConnectivityManager().use { manager ->
            manager.setConnectionStatus(status)
        }
    }
}
