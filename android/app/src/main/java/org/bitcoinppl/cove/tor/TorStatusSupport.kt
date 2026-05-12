package org.bitcoinppl.cove.tor

import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.withContext
import org.json.JSONObject
import java.net.InetSocketAddress
import java.net.Proxy
import java.net.Socket
import java.net.URL

data class TorBootstrapSnapshot(
    val percent: Int,
    val step: String,
    val isReady: Boolean,
    val hasError: Boolean,
    val lastLine: String,
)

data class TorApiSnapshot(
    val isTor: Boolean,
    val ip: String?,
    val raw: String,
)

private val bootstrapPercentRegex = Regex("""\b(100|[0-9]{1,2})%\b""")
private val artiStatusRegex = Regex("""arti_client::status]\s*(100|[0-9]{1,2})%:\s*(.+)$""")
private val missingCountRegex = Regex("""missing\s+(\d+)""")
private val missingFractionRegex = Regex("""missing\s+(\d+)\s*/\s*(\d+)""")
private val torApiBooleanRegex =
    Regex(""""is_?tor"\s*:\s*true""", RegexOption.IGNORE_CASE)
private val torApiIpRegex =
    Regex(""""ip"\s*:\s*"([^"]+)"""", RegexOption.IGNORE_CASE)

private fun isRustTorLog(line: String): Boolean {
    val trimmed = line.trim()
    return trimmed.startsWith("[INFO ") ||
        trimmed.startsWith("[WARN ") ||
        trimmed.startsWith("[ERROR ") ||
        trimmed.startsWith("[DEBUG ")
}

fun deriveBuiltInBootstrapSnapshot(logLines: List<String>): TorBootstrapSnapshot {
    if (logLines.isEmpty()) {
        return TorBootstrapSnapshot(
            percent = 0,
            step = "Waiting for Tor runtime",
            isReady = false,
            hasError = false,
            lastLine = "No Tor logs yet",
        )
    }

    val rustLogs = logLines.filter(::isRustTorLog)
    if (rustLogs.isEmpty()) {
        return TorBootstrapSnapshot(
            percent = 0,
            step = "Waiting for Tor runtime",
            isReady = false,
            hasError = false,
            lastLine = logLines.last(),
        )
    }

    val restartMarkers =
        listOf(
            "built-in tor endpoint requested without cache; launching proxy",
            "built-in tor launch initiated",
            "starting built-in tor runtime thread",
        )
    val restartIndex =
        rustLogs.indexOfLast { line ->
            restartMarkers.any { marker -> line.contains(marker, ignoreCase = true) }
        }
    val scopedLogs = if (restartIndex >= 0) rustLogs.drop(restartIndex) else rustLogs

    var percent = 0
    var step = "Starting Tor"
    var ready = false
    var hasError = false
    var initialMissingMicrodescriptors: Int? = null

    scopedLogs.forEach { line ->
        val lowered = line.lowercase()
        artiStatusRegex.find(line)?.let { match ->
            val found = match.groupValues[1].toInt().coerceIn(0, 100)
            val message = match.groupValues[2]
            percent = found
            step = "$found%: $message"
            if (found >= 100) {
                ready = true
            }
            return@forEach
        }

        bootstrapPercentRegex.find(line)?.groupValues?.get(1)?.toIntOrNull()?.let { found ->
            if (found > percent) {
                percent = found
            }
        }

        when {
            "built-in tor launch initiated" in lowered ||
                "starting built-in tor runtime thread" in lowered -> {
                percent = maxOf(percent, 3)
                step = "Launching runtime"
            }
            "built-in tor runtime created" in lowered ||
                "launching arti socks proxy task" in lowered -> {
                percent = maxOf(percent, 8)
                step = "Starting SOCKS proxy"
            }
            "listening on" in lowered &&
                ("127.0.0.1:" in lowered || "[::1]:" in lowered) -> {
                percent = maxOf(percent, 15)
                step = "SOCKS listener ready"
            }
            "looking for a consensus" in lowered -> {
                percent = maxOf(percent, 22)
                step = "Looking for consensus"
            }
            "downloading certificates for consensus" in lowered -> {
                percent = maxOf(percent, 35)
                step = "Downloading consensus certificates"
            }
            "downloading microdescriptors" in lowered -> {
                step = "Downloading microdescriptors"
                val missingFraction =
                    missingFractionRegex.find(lowered)
                        ?.groupValues
                        ?.drop(1)
                        ?.mapNotNull { value -> value.toIntOrNull() }
                val missing =
                    (missingFraction?.getOrNull(0))
                        ?: missingCountRegex.find(lowered)
                        ?.groupValues
                        ?.getOrNull(1)
                        ?.toIntOrNull()
                if (missing != null) {
                    val baselineMissing = initialMissingMicrodescriptors
                    val baseline =
                        if (missingFraction?.getOrNull(1) != null && missingFraction[1] > 0) {
                            val total = missingFraction[1]
                            if (baselineMissing == null || total > baselineMissing) {
                                initialMissingMicrodescriptors = total
                                total
                            } else {
                                baselineMissing
                            }
                        } else if (baselineMissing == null || missing > baselineMissing) {
                            initialMissingMicrodescriptors = missing
                            missing
                        } else {
                            baselineMissing
                        }.coerceAtLeast(1)
                    val completedRatio =
                        ((baseline - missing).coerceAtLeast(0)).toDouble() / baseline.toDouble()
                    val dynamicPercent = (45 + (completedRatio * 46.0).toInt()).coerceIn(45, 91)
                    percent = maxOf(percent, dynamicPercent)
                } else {
                    percent = maxOf(percent, 45)
                }
            }
            "marked consensus usable" in lowered -> {
                percent = maxOf(percent, 93)
                step = "Building circuits"
            }
            "enough information to build circuits" in lowered -> {
                percent = maxOf(percent, 96)
                step = "Building circuits"
            }
            "directory is complete" in lowered -> {
                percent = 100
                step = "Tor ready"
                ready = true
            }
            "sufficiently bootstrapped; proxy now functional" in lowered -> {
                percent = maxOf(percent, 97)
                step = "Circuits available, finishing directory"
            }
        }

        val benignReloadXdgWarning =
            "arti::reload_cfg" in lowered &&
                ("xdg project directories" in lowered ||
                    "unable to determine home directory" in lowered ||
                    "cache_dir" in lowered)

        val fatalBootstrapSignals =
            listOf(
                "built-in tor bootstrap failed",
                "built-in tor proxy exited",
                "failed to initialize built-in tor runtime",
                "failed to create built-in tor runtime",
                "built-in tor socks listener not ready",
                "can't find path for port_info_file",
                "operation not supported because arti feature disabled",
            )

        if (!benignReloadXdgWarning &&
            fatalBootstrapSignals.any { signal -> signal in lowered }
        ) {
            hasError = true
        }
    }

    if (ready) {
        hasError = false
    }

    if (!ready && percent >= 100) {
        percent = 99
    }

    val lastLine = scopedLogs.lastOrNull() ?: rustLogs.last()
    return TorBootstrapSnapshot(
        percent = percent.coerceIn(0, 100),
        step = step,
        isReady = ready,
        hasError = hasError,
        lastLine = lastLine,
    )
}

suspend fun testSocksEndpoint(
    host: String,
    port: Int,
    timeoutMs: Int = 3000,
): Result<Unit> =
    try {
        withContext(Dispatchers.IO) {
            Socket().use { socket ->
                socket.connect(InetSocketAddress(host, port), timeoutMs)
            }
        }
        Result.success(Unit)
    } catch (error: CancellationException) {
        throw error
    } catch (error: Exception) {
        Result.failure(error)
    }

suspend fun testTorApiThroughSocks(
    host: String,
    port: Int,
    timeoutMs: Int = 15000,
): Result<TorApiSnapshot> =
    try {
        val snapshot =
            withContext(Dispatchers.IO) {
                val proxy = Proxy(Proxy.Type.SOCKS, InetSocketAddress(host, port))
                val connection = URL("https://check.torproject.org/api/ip").openConnection(proxy)
                connection.connectTimeout = timeoutMs
                connection.readTimeout = timeoutMs

                val raw =
                    connection.getInputStream().bufferedReader().use { reader ->
                        reader.readText()
                    }
                val json = runCatching { JSONObject(raw) }.getOrNull()
                val isTor =
                    json?.caseInsensitiveBoolean("IsTor")
                        ?: json?.caseInsensitiveBoolean("is_tor")
                        ?: torApiBooleanRegex.containsMatchIn(raw)
                val ip =
                    json?.caseInsensitiveString("IP")
                        ?: torApiIpRegex.find(raw)?.groupValues?.getOrNull(1)

                TorApiSnapshot(isTor = isTor, ip = ip, raw = raw)
            }
        Result.success(snapshot)
    } catch (error: CancellationException) {
        throw error
    } catch (error: Exception) {
        Result.failure(error)
    }

private fun JSONObject.caseInsensitiveBoolean(key: String): Boolean? {
    val actualKey = keys().asSequence().firstOrNull { it.equals(key, ignoreCase = true) }
    return actualKey?.let { optBoolean(it) }
}

private fun JSONObject.caseInsensitiveString(key: String): String? {
    val actualKey = keys().asSequence().firstOrNull { it.equals(key, ignoreCase = true) }
    return actualKey?.let { optString(it).takeIf(String::isNotEmpty) }
}
