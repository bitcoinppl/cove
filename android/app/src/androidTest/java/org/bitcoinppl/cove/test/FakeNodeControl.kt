package org.bitcoinppl.cove.test

import org.json.JSONArray
import org.json.JSONObject
import java.io.BufferedInputStream
import java.io.ByteArrayOutputStream
import java.io.InputStream
import java.net.InetSocketAddress
import java.net.Socket

/** Esplora URL of the fake node the runner forwards into the device with `adb reverse` */
const val FAKE_NODE_ESPLORA_URL = "http://127.0.0.1:39001"

/** An output on the fake node's chain */
data class FakeNodeOutPoint(
    val txid: String,
    val vout: Int,
) {
    override fun toString(): String = "$txid:$vout"

    companion object {
        fun parse(value: String): FakeNodeOutPoint {
            val (txid, vout) = value.split(":", limit = 2)

            return FakeNodeOutPoint(txid, vout.toInt())
        }
    }
}

/** A funding output returned by `/fund` and `/fund-wallet` */
data class FakeNodeFundedOutput(
    val outpoint: FakeNodeOutPoint,
    val sats: Long,
    val address: String,
)

/** A transaction the app broadcast, as logged by `/broadcasts` */
data class FakeNodeBroadcast(
    val txid: String,
    val inputs: List<FakeNodeOutPoint>,
    val outputs: List<FakeNodeTxOut>,
)

data class FakeNodeTxOut(
    val address: String?,
    val sats: Long,
)

/**
 * Client for the cove-fake-node control API (`rust/crates/cove-fake-node/README.md`)
 *
 * Start the node with `cargo xtask fake-node run --android -- <command>` or
 * `just android-instrumented-ci`, which forward its ports into the device. The client speaks
 * HTTP/1.1 over a plain socket because instrumentation runs inside the app process, whose
 * network security policy blocks cleartext `HttpURLConnection` traffic
 */
class FakeNodeControl(
    private val host: String = "127.0.0.1",
    private val port: Int = 39_000,
) {
    /** Replaces the chain with a fresh one and clears the broadcast log */
    fun reset() {
        request("POST", "/reset", JSONObject())
    }

    /** Funds the BIP84 receive addresses `index` of the mnemonic, one confirmed transaction per amount */
    fun fundWallet(
        mnemonic: String,
        satsByIndex: List<Pair<Int, Long>>,
    ): List<FakeNodeFundedOutput> {
        val outputs = JSONArray()
        satsByIndex.forEach { (index, sats) ->
            outputs.put(JSONObject().put("index", index).put("sats", sats))
        }

        val body =
            JSONObject()
                .put("mnemonic", mnemonic)
                .put("outputs", outputs)
                .put("confirm", true)
        val response = JSONObject(request("POST", "/fund-wallet", body))

        return response.getJSONArray("outputs").objects().map { output ->
            FakeNodeFundedOutput(
                outpoint = FakeNodeOutPoint(output.getString("txid"), output.getInt("vout")),
                sats = output.getLong("sats"),
                address = output.getString("address"),
            )
        }
    }

    /** Mines blocks, the first confirming every mempool transaction, and returns the new tip */
    fun mine(blocks: Int = 1): Int = JSONObject(request("POST", "/mine", JSONObject().put("blocks", blocks))).getInt("tip")

    fun broadcasts(): List<FakeNodeBroadcast> =
        JSONArray(request("GET", "/broadcasts")).objects().map { broadcast ->
            FakeNodeBroadcast(
                txid = broadcast.getString("txid"),
                inputs = broadcast.getJSONArray("inputs").strings().map(FakeNodeOutPoint::parse),
                outputs =
                    broadcast.getJSONArray("outputs").objects().map { output ->
                        FakeNodeTxOut(
                            address = output.optString("address").takeUnless { output.isNull("address") },
                            sats = output.getLong("sats"),
                        )
                    },
            )
        }

    fun isSpent(outpoint: FakeNodeOutPoint): Boolean =
        JSONObject(request("GET", "/outspend/${outpoint.txid}/${outpoint.vout}")).getBoolean("spent")

    private fun request(
        method: String,
        path: String,
        body: JSONObject? = null,
    ): String {
        val bodyBytes = body?.toString()?.toByteArray(Charsets.UTF_8) ?: ByteArray(0)

        val (status, responseBody) =
            try {
                Socket().use { socket ->
                    socket.connect(InetSocketAddress(host, port), CONNECT_TIMEOUT_MS)
                    socket.soTimeout = READ_TIMEOUT_MS

                    val head =
                        "$method $path HTTP/1.1\r\n" +
                            "host: $host:$port\r\n" +
                            "content-type: application/json\r\n" +
                            "content-length: ${bodyBytes.size}\r\n" +
                            "connection: close\r\n\r\n"
                    socket.getOutputStream().apply {
                        write(head.toByteArray(Charsets.US_ASCII))
                        write(bodyBytes)
                        flush()
                    }

                    readResponse(BufferedInputStream(socket.getInputStream()))
                }
            } catch (error: java.io.IOException) {
                throw IllegalStateException(
                    "cove-fake-node control API at $host:$port is unreachable; run the test through " +
                        "`just android-instrumented-ci` or `cargo xtask fake-node run --android -- <command>`",
                    error,
                )
            }

        check(status == 200) { "$method $path returned $status: $responseBody" }

        return responseBody
    }

    // the server always sends content-length, so the body is read by length rather than to EOF
    private fun readResponse(input: InputStream): Pair<Int, String> {
        val statusLine = input.readHttpLine()
        val status = statusLine.split(" ").getOrNull(1)?.toIntOrNull() ?: error("bad status line: $statusLine")
        var contentLength = 0

        while (true) {
            val header = input.readHttpLine()
            if (header.isEmpty()) break

            val (name, value) = header.split(":", limit = 2).let { it[0] to it.getOrElse(1) { "" } }
            if (name.trim().equals("content-length", ignoreCase = true)) {
                contentLength = value.trim().toInt()
            }
        }

        val body = ByteArray(contentLength)
        var read = 0
        while (read < contentLength) {
            val count = input.read(body, read, contentLength - read)
            check(count >= 0) { "connection closed after $read of $contentLength body bytes" }
            read += count
        }

        return status to String(body, Charsets.UTF_8)
    }

    private fun InputStream.readHttpLine(): String {
        val line = ByteArrayOutputStream()

        while (true) {
            val byte = read()
            check(byte >= 0) { "connection closed mid-line" }
            if (byte == '\n'.code) break
            if (byte != '\r'.code) line.write(byte)
        }

        return line.toString(Charsets.US_ASCII.name())
    }

    private fun JSONArray.objects(): List<JSONObject> = (0 until length()).map(::getJSONObject)

    private fun JSONArray.strings(): List<String> = (0 until length()).map(::getString)

    private companion object {
        const val CONNECT_TIMEOUT_MS = 5_000
        const val READ_TIMEOUT_MS = 30_000
    }
}
