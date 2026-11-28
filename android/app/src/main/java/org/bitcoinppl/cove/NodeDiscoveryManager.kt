package org.bitcoinppl.cove

import android.content.Context
import android.net.wifi.WifiManager
import androidx.compose.runtime.Stable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.launch
import org.bitcoinppl.cove_core.NodeDiscoveryManagerReconciler
import org.bitcoinppl.cove_core.NodeDiscoveryReconcileMessage
import org.bitcoinppl.cove_core.NodeDiscoveryState
import org.bitcoinppl.cove_core.RustNodeDiscoveryManager
import java.io.Closeable
import java.util.concurrent.atomic.AtomicBoolean

/**
 * Runs node settings' searches of the local network for Electrum servers
 *
 * The screen owns one while it is shown. Rust runs each search and lists every server as its
 * check passes; this hands it the Wi-Fi and Ethernet interfaces to search, and holds the multicast
 * lock that lets mDNS answers through the Wi-Fi driver's filter on Android 12 and older while a
 * search runs
 */
@Stable
class NodeDiscoveryManager internal constructor(
    context: Context,
) : NodeDiscoveryManagerReconciler,
    Closeable {
    private val tag = "NodeDiscoveryManager"
    private val lan = LanNodeDiscovery(context)
    private val multicastLock =
        context.applicationContext
            .getSystemService(WifiManager::class.java)
            ?.createMulticastLock(MULTICAST_LOCK_TAG)
            ?.apply { setReferenceCounted(false) }

    private val mainScope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)
    private val isClosed = AtomicBoolean(false)
    private val rustGuard =
        RustHandleGuard(
            ownerName = "NodeDiscoveryManager",
            handleName = "RustNodeDiscoveryManager",
            isClosed = isClosed,
        ) {
            Log.w(tag, it)
        }

    private val rust = RustNodeDiscoveryManager()

    var state by mutableStateOf<NodeDiscoveryState>(NodeDiscoveryState.Idle)
        private set

    init {
        rust.listenForUpdates(this)
        state = rust.state()
    }

    /** Searches the Wi-Fi and Ethernet networks the phone is on, unless a search is running */
    fun start() {
        if (state is NodeDiscoveryState.Searching) return

        // TODO(targetSdk 37): declare ACCESS_LOCAL_NETWORK and request it here before searching,
        // since apps targeting SDK 37 can't reach devices on the local network without it
        multicastLock?.acquire()
        val interfaces = lan.lanInterfaces()
        rustGuard.withHandleOr(rust, Unit) { start(interfaces) }
    }

    /** Stops the running search, keeping the servers it found */
    fun stop() {
        rustGuard.withHandleOr(rust, Unit) { stop() }
    }

    override fun reconcile(message: NodeDiscoveryReconcileMessage) {
        mainScope.launch { apply(message) }
    }

    override fun reconcileMany(messages: List<NodeDiscoveryReconcileMessage>) {
        mainScope.launch { messages.forEach(::apply) }
    }

    private fun apply(message: NodeDiscoveryReconcileMessage) {
        val current = state

        state =
            when (message) {
                NodeDiscoveryReconcileMessage.Started -> NodeDiscoveryState.Searching(emptyList())

                is NodeDiscoveryReconcileMessage.Found -> {
                    if (current !is NodeDiscoveryState.Searching) return

                    val found = current.found.toMutableList()
                    found.add(message.index.toInt().coerceAtMost(found.size), message.server)
                    NodeDiscoveryState.Searching(found)
                }

                NodeDiscoveryReconcileMessage.Stopped -> NodeDiscoveryState.Stopped(found(current))

                NodeDiscoveryReconcileMessage.Finished -> NodeDiscoveryState.Finished(found(current))

                is NodeDiscoveryReconcileMessage.Failed -> NodeDiscoveryState.Failed(message.error)
            }

        if (state !is NodeDiscoveryState.Searching) releaseMulticastLock()
    }

    private fun releaseMulticastLock() {
        multicastLock?.takeIf { it.isHeld }?.release()
    }

    private fun found(state: NodeDiscoveryState) = (state as? NodeDiscoveryState.Searching)?.found.orEmpty()

    override fun close() {
        rustGuard.closeOnce {
            mainScope.cancel()
            releaseMulticastLock()

            // dropping the Rust manager cancels a running search
            rust.close()
        }
    }

    private companion object {
        const val MULTICAST_LOCK_TAG = "cove-node-discovery"
    }
}
