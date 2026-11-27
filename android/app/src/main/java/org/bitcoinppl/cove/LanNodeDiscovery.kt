package org.bitcoinppl.cove

import android.content.Context
import android.net.ConnectivityManager
import android.net.NetworkCapabilities
import org.bitcoinppl.cove_core.LanBinding
import org.bitcoinppl.cove_core.LanInterface
import java.net.Inet4Address

/**
 * The Wi-Fi and Ethernet networks a node search may search
 *
 * Rust owns the search and keeps it on the local network. This supplies which interfaces are
 * Wi-Fi or Ethernet, and the network handle Rust binds every socket to so a VPN can't carry the
 * search
 */
class LanNodeDiscovery(
    context: Context,
) {
    private val appContext = context.applicationContext

    /** The IPv4 addresses of every connected Wi-Fi or Ethernet network, never cellular or a VPN */
    fun lanInterfaces(): List<LanInterface> {
        val connectivityManager =
            appContext.getSystemService(ConnectivityManager::class.java) ?: return emptyList()

        // allNetworks is the only synchronous way to see the Wi-Fi network under an active VPN,
        // which activeNetwork hides
        @Suppress("DEPRECATION")
        val networks = connectivityManager.allNetworks

        return networks
            .filter { network ->
                val capabilities = connectivityManager.getNetworkCapabilities(network) ?: return@filter false
                val isLocal =
                    capabilities.hasTransport(NetworkCapabilities.TRANSPORT_WIFI) ||
                        capabilities.hasTransport(NetworkCapabilities.TRANSPORT_ETHERNET)

                isLocal && !capabilities.hasTransport(NetworkCapabilities.TRANSPORT_VPN)
            }.flatMap { network ->
                val binding = LanBinding.AndroidNetwork(network.networkHandle.toULong())
                val linkAddresses = connectivityManager.getLinkProperties(network)?.linkAddresses.orEmpty()

                linkAddresses.mapNotNull { linkAddress ->
                    val address = linkAddress.address as? Inet4Address ?: return@mapNotNull null
                    val hostAddress = address.hostAddress ?: return@mapNotNull null

                    LanInterface(
                        ipv4Address = hostAddress,
                        prefixLength = linkAddress.prefixLength.toUByte(),
                        binding = binding,
                    )
                }
            }.distinct()
    }
}
