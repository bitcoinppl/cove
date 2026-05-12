package org.bitcoinppl.cove.flows.SettingsFlow

import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import org.bitcoinppl.cove_core.Node
import org.bitcoinppl.cove_core.NodeSelection
import org.bitcoinppl.cove_core.NodeSelector
import java.net.URI
import java.security.MessageDigest

fun isOnionNodeUrl(url: String): Boolean {
    return try {
        val normalized = if (url.contains("://")) url else "tcp://$url"
        URI(normalized).host?.endsWith(".onion", ignoreCase = true) == true
    } catch (_: Exception) {
        false
    }
}

fun redactedEndpointForLog(endpoint: String): String {
    return "id=${shortLogId(endpoint)}, onion=${isOnionNodeUrl(endpoint)}"
}

fun redactedProxyForLog(host: String, port: UShort): String {
    return "id=${shortLogId("$host:$port")}, port=$port"
}

fun redactedNodeForLog(node: Node): String {
    return "apiType=${node.apiType}, ${redactedEndpointForLog(node.url)}"
}

private fun shortLogId(value: String): String {
    val digest = MessageDigest.getInstance("SHA-256")
        .digest(value.toByteArray(Charsets.UTF_8))
    return digest.take(4).joinToString("") { byte -> "%02x".format(byte) }
}

suspend fun switchToFirstClearnetPresetNode(nodeSelector: NodeSelector): Result<Node> =
    runCatching {
        withContext(Dispatchers.IO) {
            val fallbackNode =
                nodeSelector
                    .nodeList()
                    .asSequence()
                    .mapNotNull { selection -> (selection as? NodeSelection.Preset)?.v1 }
                    .firstOrNull { node -> !isOnionNodeUrl(node.url) }
                    ?: throw IllegalStateException("No clearnet preset node available for fallback")

            nodeSelector.selectPresetNode(fallbackNode.name)
        }
    }
