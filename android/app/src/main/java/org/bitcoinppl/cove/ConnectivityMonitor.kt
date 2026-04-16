package org.bitcoinppl.cove

import android.content.Context
import android.net.ConnectivityManager
import android.net.Network
import android.net.NetworkCapabilities
import org.bitcoinppl.cove_core.RustConnectivityManager
import org.bitcoinppl.cove_core.device.ConnectivityAccess

class ConnectivityMonitor(
    context: Context,
) : ConnectivityAccess {
    private val connectivityManager =
        context.getSystemService(ConnectivityManager::class.java)
            ?: error("ConnectivityManager unavailable")
    private var started = false

    private val callback =
        object : ConnectivityManager.NetworkCallback() {
            override fun onAvailable(network: Network) {
                pushConnectivityState(true)
            }

            override fun onLost(network: Network) {
                pushConnectivityState(isConnected())
            }

            override fun onUnavailable() {
                pushConnectivityState(false)
            }
        }

    override fun isConnected(): Boolean {
        val network = connectivityManager.activeNetwork ?: return false
        val capabilities = connectivityManager.getNetworkCapabilities(network) ?: return false
        return capabilities.hasCapability(NetworkCapabilities.NET_CAPABILITY_INTERNET) &&
            capabilities.hasCapability(NetworkCapabilities.NET_CAPABILITY_VALIDATED)
    }

    fun start() {
        if (started) return
        started = true

        pushConnectivityState(isConnected())
        connectivityManager.registerDefaultNetworkCallback(callback)
    }

    private fun pushConnectivityState(connected: Boolean) {
        RustConnectivityManager().use { rustConnectivityManager ->
            rustConnectivityManager.setConnectionState(connected)
        }
    }
}
