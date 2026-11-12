package org.bitcoinppl.cove

import android.content.Context
import android.net.ConnectivityManager
import android.net.NetworkCapabilities
import android.net.wifi.WifiManager
import org.bitcoinppl.cove_core.DiscoveredNode
import org.bitcoinppl.cove_core.LanBinding
import org.bitcoinppl.cove_core.LanInterface
import org.bitcoinppl.cove_core.NodeSelector
import java.net.Inet4Address

/**
 * Finds Electrum servers on the Wi-Fi or Ethernet network the phone is on
 *
 * Rust owns the search and keeps it on the local network. This class supplies the platform
 * pieces: which interfaces are Wi-Fi or Ethernet, the network handle Rust binds every socket to so
 * a VPN can't carry the search, and the multicast lock that lets mDNS answers through the Wi-Fi
 * driver's filter on Android 12 and older
 */
class LanNodeDiscovery(
    context: Context,
) {
    private val appContext = context.applicationContext

    /**
     * Searches the local network, failing with `NodeDiscoveryException.NoLocalNetwork` off Wi-Fi
     * and `LocalNetworkBlocked` when a VPN that doesn't allow local network access covers the app
     */
    @Suppress("RedundantSuspendModifier") // detekt can't see that the generated discoverLanNodes suspends
    suspend fun discover(nodeSelector: NodeSelector): List<DiscoveredNode> {
        val wifiManager = appContext.getSystemService(WifiManager::class.java)
        val multicastLock =
            wifiManager?.createMulticastLock(MULTICAST_LOCK_TAG)?.apply {
                setReferenceCounted(false)
                acquire()
            }

        try {
            return nodeSelector.discoverLanNodes(lanInterfaces())
        } finally {
            multicastLock?.release()
        }
    }

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

    private companion object {
        const val MULTICAST_LOCK_TAG = "cove-node-discovery"
    }
}
