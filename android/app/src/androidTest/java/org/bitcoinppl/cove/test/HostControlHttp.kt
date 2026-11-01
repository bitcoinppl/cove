package org.bitcoinppl.cove.test

import org.json.JSONObject
import java.io.BufferedInputStream
import java.io.ByteArrayOutputStream
import java.io.IOException
import java.io.InputStream
import java.net.InetSocketAddress
import java.net.Socket

/**
 * Sends JSON requests to a control API the test runner serves on the host and forwards into the
 * device with `adb reverse`
 *
 * Speaks HTTP/1.1 over a plain socket because instrumentation runs inside the app process, whose
 * network security policy blocks cleartext `HttpURLConnection` traffic
 */
class HostControlHttp(
    private val host: String,
    private val port: Int,
    private val unreachableMessage: () -> String,
) {
    /** Returns the response body, failing on any status but 200 */
    fun request(
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
            } catch (error: IOException) {
                throw IllegalStateException(unreachableMessage(), error)
            }

        check(status == 200) { "$method $path returned $status: $responseBody" }

        return responseBody
    }

    // the servers always send content-length, so the body is read by length rather than to EOF
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

    private companion object {
        const val CONNECT_TIMEOUT_MS = 5_000

        // starting the card emulator can take a while the first time it installs its packages
        const val READ_TIMEOUT_MS = 120_000
    }
}
