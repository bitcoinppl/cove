package org.bitcoinppl.cove.uitest

import android.system.Os
import java.io.EOFException
import java.io.File
import java.io.IOException
import java.io.InputStream
import java.io.OutputStream
import java.net.InetSocketAddress
import java.net.Socket
import java.nio.ByteBuffer
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.flow
import kotlinx.coroutines.flow.flowOn
import kotlinx.coroutines.withContext
import org.bitcoinppl.cove.Log
import org.bitcoinppl.cove_core.TapcardTransportProtocol
import org.bitcoinppl.cove_core.TransportException
import org.json.JSONException
import org.json.JSONObject

/**
 * Debug-only stand-ins for the camera and NFC that UI tests drive.
 *
 * The release source set has the same entry points, each reporting that no
 * seam is configured, so none of this code is in a release build.
 *
 * Both seams read their configuration from the process environment, which an
 * instrumented test sets with `Os.setenv` before it launches the activity:
 * - `COVE_UI_TEST_QR_SCRIPT` names a file an open QR scanner polls. The test
 *   writes `{"frames": ["…", …]}` there, atomically; the scanner takes the file
 *   and feeds the frames one at a time through the path a camera detection
 *   takes. Writing the file again feeds more frames to the same scan
 * - `COVE_UI_TEST_TAPSIGNER_BRIDGE` is `host:port` of the TAPSIGNER bridge that
 *   `xtask ui-test-host` serves, which `adb reverse` forwards to the device.
 *   TAPSIGNER commands and NFC tag reads talk to it instead of an NFC tag
 */
// debug-only test seam with no caller that could inject a dispatcher
@Suppress("InjectDispatcher")
internal object UiTestSeams {
    private const val TAG = "UiTestSeams"
    private const val QR_SCRIPT_ENV = "COVE_UI_TEST_QR_SCRIPT"
    private const val TAPSIGNER_BRIDGE_ENV = "COVE_UI_TEST_TAPSIGNER_BRIDGE"
    private const val QR_FRAME_INTERVAL_MS = 150L
    private const val QR_POLL_INTERVAL_MS = 200L

    // `{"cmd": "nfc"}` in a CBOR APDU, which returns the URL a card's NDEF record holds
    private val NFC_URL_APDU =
        byteArrayOf(
            0x00,
            0xCB.toByte(),
            0x00,
            0x00,
            0x09,
            0xA1.toByte(),
            0x63,
            0x63,
            0x6D,
            0x64,
            0x63,
            0x6E,
            0x66,
            0x63,
        )

    /** Scripted QR frames to scan instead of camera frames, or null to use the camera */
    fun qrScriptFrames(): Flow<String>? {
        val path = Os.getenv(QR_SCRIPT_ENV) ?: return null
        val script = File(path)

        return flow {
            while (true) {
                for (frame in takeFrames(script)) {
                    emit(frame)

                    // camera frames arrive spaced out, and the scan view updates its
                    // progress between them
                    delay(QR_FRAME_INTERVAL_MS)
                }

                delay(QR_POLL_INTERVAL_MS)
            }
        }.flowOn(Dispatchers.IO)
    }

    /** Whether TAPSIGNER commands and NFC reads go to the card emulator bridge */
    val tapSignerBridgeConfigured: Boolean
        get() = tapSignerBridgeAddress() != null

    /**
     * Runs one TAPSIGNER session through the bridge, as a card held to the phone
     *
     * Call only when [tapSignerBridgeConfigured] is true
     */
    suspend fun <T> withTapSignerTransport(
        onMessage: (String) -> Unit,
        block: suspend (TapcardTransportProtocol) -> T,
    ): T {
        val address =
            tapSignerBridgeAddress()
                ?: error("$TAPSIGNER_BRIDGE_ENV is not set")

        return BridgeTransport(address, onMessage).use { block(it) }
    }

    /** The URL a TAPSIGNER's NDEF record holds, as an NFC tag read returns it */
    suspend fun readTapSignerNdefUrl(): String =
        withTapSignerTransport(onMessage = {}) {
            val response = it.transmitApdu(NFC_URL_APDU)
            cborUrl(response) ?: throw IOException("The card emulator returned no NFC URL")
        }

    private fun takeFrames(script: File): List<String> {
        if (!script.exists()) return emptyList()

        return try {
            val json = JSONObject(script.readText())
            val frames = json.getJSONArray("frames")
            Log.d(TAG, "Took ${frames.length()} QR frames from ${script.path}")
            List(frames.length()) { frames.getString(it) }
        } catch (e: IOException) {
            Log.e(TAG, "UI test QR script at ${script.path} could not be read", e)
            emptyList()
        } catch (e: JSONException) {
            Log.e(TAG, "UI test QR script at ${script.path} is not valid", e)
            emptyList()
        } finally {
            script.delete()
        }
    }

    private fun tapSignerBridgeAddress(): InetSocketAddress? {
        val value = Os.getenv(TAPSIGNER_BRIDGE_ENV) ?: return null
        val port = value.substringAfterLast(':').toIntOrNull()
        if (port == null) Log.e(TAG, "$TAPSIGNER_BRIDGE_ENV must be host:port, got $value")

        return port?.let { InetSocketAddress.createUnresolved(value.substringBeforeLast(':'), it) }
    }

    /** Reads `{"url": text}` from a card response, ignoring the trailing status word */
    private fun cborUrl(response: ByteArray): String? {
        val prefix = byteArrayOf(0xA1.toByte(), 0x63, 0x75, 0x72, 0x6C)
        val hasPrefix =
            response.size > prefix.size && response.copyOfRange(0, prefix.size).contentEquals(prefix)
        if (!hasPrefix) return null

        // a CBOR text string header: major type 3 with an inline, 1-byte or 2-byte length,
        // as (offset of the text, text length)
        val start = prefix.size
        fun byteAt(index: Int) = response[index].toUByte().toInt()

        val header = byteAt(start)
        val span =
            when {
                header in 0x60..0x77 -> start + 1 to header - 0x60
                header == 0x78 && response.size > start + 1 -> start + 2 to byteAt(start + 1)
                header == 0x79 && response.size > start + 2 ->
                    start + 3 to ((byteAt(start + 1) shl 8) or byteAt(start + 2))
                else -> null
            }

        return span
            ?.takeIf { (offset, length) -> response.size >= offset + length }
            ?.let { (offset, length) -> String(response, offset, length, Charsets.UTF_8) }
    }

    /**
     * Sends a TAPSIGNER session's APDUs over one bridge connection.
     *
     * Each APDU goes out as a frame with a 4-byte big-endian length and the
     * card's response comes back the same way. The bridge closing the
     * connection looks like the card leaving the NFC field.
     */
    private class BridgeTransport(
        private val address: InetSocketAddress,
        private val onMessage: (String) -> Unit,
    ) : TapcardTransportProtocol,
        AutoCloseable {
        private var socket: Socket? = null
        private var currentMessage = ""

        override fun setMessage(message: String) {
            currentMessage = message
            onMessage(currentMessage)
        }

        override fun appendMessage(message: String) {
            currentMessage += message
            onMessage(currentMessage)
        }

        override suspend fun transmitApdu(commandApdu: ByteArray): ByteArray =
            withContext(Dispatchers.IO) {
                try {
                    val socket = connectedSocket()
                    writeFrame(socket.getOutputStream(), commandApdu)
                    readFrame(socket.getInputStream())
                } catch (e: IOException) {
                    Log.e(TAG, "TAPSIGNER bridge exchange failed", e)
                    close()

                    // the message a real tag connection loss reports
                    throw TransportException.Transport(
                        "Tag connection lost, please hold your phone still and try again",
                    )
                }
            }

        @Synchronized
        override fun close() {
            runCatching { socket?.close() }
            socket = null
        }

        @Synchronized
        private fun connectedSocket(): Socket =
            socket ?: Socket().also {
                it.soTimeout = RECEIVE_TIMEOUT_MS
                it.connect(InetSocketAddress(address.hostString, address.port), CONNECT_TIMEOUT_MS)
                socket = it
            }

        private fun writeFrame(
            output: OutputStream,
            frame: ByteArray,
        ) {
            // ByteBuffer is big-endian by default, as the bridge expects
            output.write(ByteBuffer.allocate(Int.SIZE_BYTES).putInt(frame.size).array())
            output.write(frame)
            output.flush()
        }

        private fun readFrame(input: InputStream): ByteArray {
            val length = ByteBuffer.wrap(readExactly(input, Int.SIZE_BYTES)).int
            return readExactly(input, length)
        }

        private fun readExactly(
            input: InputStream,
            count: Int,
        ): ByteArray {
            val buffer = ByteArray(count)
            var offset = 0
            while (offset < count) {
                val read = input.read(buffer, offset, count - offset)
                if (read < 0) throw EOFException("the bridge closed the connection")
                offset += read
            }

            return buffer
        }

        private companion object {
            const val CONNECT_TIMEOUT_MS = 5_000
            const val RECEIVE_TIMEOUT_MS = 30_000
        }
    }
}
