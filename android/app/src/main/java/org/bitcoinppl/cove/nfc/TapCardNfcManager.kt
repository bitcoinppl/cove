package org.bitcoinppl.cove.nfc

import android.app.Activity
import android.nfc.NfcAdapter
import android.nfc.Tag
import android.nfc.tech.IsoDep
import android.util.Log
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.suspendCancellableCoroutine
import kotlinx.coroutines.withTimeoutOrNull
import org.bitcoinppl.cove_core.*
import org.bitcoinppl.cove_core.TapcardTransportProtocol
import org.bitcoinppl.cove_core.createTapSignerReader
import kotlin.coroutines.resume
import kotlin.coroutines.resumeWithException

/**
 * Manages NFC operations for TapCard (TapSigner/SatsCard)
 * singleton that's initialized with Activity context from MainActivity
 * ported from iOS TapCardNFC class
 */
class TapCardNfcManager private constructor() {
    private val tag = "TapCardNfcManager"
    private var activity: Activity? = null
    private var nfcAdapter: NfcAdapter? = null

    // current operation state
    private var currentCmd: TapSignerCmd? = null
    private var tagDetected = CompletableDeferred<Tag>()
    private var isScanning = false

    companion object {
        @Volatile
        private var instance: TapCardNfcManager? = null

        fun getInstance(): TapCardNfcManager {
            return instance ?: synchronized(this) {
                instance ?: TapCardNfcManager().also { instance = it }
            }
        }
    }

    fun initialize(activity: Activity) {
        this.activity = activity
        this.nfcAdapter = NfcAdapter.getDefaultAdapter(activity)
        Log.d(tag, "TapCardNfcManager initialized")
    }

    /**
     * Perform a TapSigner command by scanning for NFC tag
     * matches iOS performTapSignerCmd pattern
     */
    suspend fun <T> performTapSignerCmd(
        cmd: TapSignerCmd,
        successResult: (TapSignerResponse?) -> T?,
    ): T {
        val activity = this.activity ?: throw Exception("NfcManager not initialized with Activity")
        val nfcAdapter = this.nfcAdapter ?: throw Exception("NFC not available on this device")

        if (!nfcAdapter.isEnabled) {
            throw Exception("NFC is disabled. Please enable it in Settings")
        }

        Log.d(tag, "Starting NFC scan for command: $cmd")

        return suspendCancellableCoroutine { continuation ->
            // reset state for new operation
            currentCmd = cmd
            tagDetected = CompletableDeferred()
            isScanning = true

            // enable reader mode for ISO14443 tags (TapSigner uses ISO7816)
            nfcAdapter.enableReaderMode(
                activity,
                { nfcTag ->
                    Log.d(tag, "NFC tag detected: ${nfcTag.techList.joinToString()}")
                    if (!tagDetected.isCompleted) {
                        tagDetected.complete(nfcTag)
                    }
                },
                NfcAdapter.FLAG_READER_NFC_A or
                    NfcAdapter.FLAG_READER_NFC_B or
                    NfcAdapter.FLAG_READER_SKIP_NDEF_CHECK or
                    NfcAdapter.FLAG_READER_NO_PLATFORM_SOUNDS,
                null,
            )

            // handle tag detection and command execution
            CoroutineScope(Dispatchers.IO).launch {
                try {
                    // wait for tag with 60 second timeout
                    val detectedTag =
                        withTimeoutOrNull(60_000) {
                            tagDetected.await()
                        } ?: throw Exception("NFC scan timed out. Please try again")

                    Log.d(tag, "Processing detected tag")

                    // get IsoDep tech (ISO7816)
                    val isoDep =
                        IsoDep.get(detectedTag)
                            ?: throw Exception("Tag doesn't support IsoDep (ISO7816)")

                    // connect to tag
                    if (!isoDep.isConnected) {
                        isoDep.connect()
                    }

                    Log.d(tag, "Connected to IsoDep tag")

                    // create transport
                    val transport = TapCardTransport(isoDep)

                    // create TapSignerReader using factory function (workaround for UniFFI async constructor limitation)
                    Log.d(tag, "Creating TapSignerReader with command using factory function")
                    val reader = createTapSignerReader(transport, cmd)

                    // run the reader and get response
                    Log.d(tag, "Running TapSignerReader")
                    val response = reader.run()

                    Log.d(tag, "TapSigner command completed successfully")

                    // clean up
                    stopScanning()
                    isoDep.close()

                    // extract result using successResult function
                    val result =
                        successResult(response)
                            ?: throw Exception("Command completed but result extraction failed")

                    continuation.resume(result)
                } catch (e: TapSignerReaderException) {
                    Log.e(tag, "TapSigner error", e)
                    stopScanning()

                    // handle specific errors
                    val errorMessage =
                        when {
                            e.message?.contains("BadAuth") == true -> "Wrong PIN, please try again"
                            e.message?.contains("connection lost") == true ->
                                "Tag connection lost, please hold your phone still"

                            else -> e.message ?: "TapSigner error occurred"
                        }

                    continuation.resumeWithException(Exception(errorMessage))
                } catch (e: Exception) {
                    Log.e(tag, "NFC operation failed", e)
                    stopScanning()
                    continuation.resumeWithException(e)
                }
            }

            // handle cancellation
            continuation.invokeOnCancellation {
                Log.d(tag, "NFC operation cancelled")
                stopScanning()
            }
        }
    }

    private fun stopScanning() {
        if (isScanning) {
            isScanning = false
            activity?.runOnUiThread {
                nfcAdapter?.disableReaderMode(activity)
                Log.d(tag, "Stopped NFC scanning")
            }
        }
    }
}

/**
 * Android NFC transport implementation
 * implements TapcardTransportProtocol for IsoDep tags
 */
private class TapCardTransport(
    private val isoDep: IsoDep,
) : TapcardTransportProtocol {
    private val tag = "TapCardTransport"

    override fun setMessage(message: String) {
        // Android NFC doesn't support updating UI message during transaction
        Log.d(tag, "Message: $message")
    }

    override fun appendMessage(message: String) {
        Log.d(tag, "Append: $message")
    }

    override suspend fun transmitApdu(commandApdu: ByteArray): ByteArray {
        Log.d(tag, "Transmitting APDU: ${commandApdu.size} bytes")

        return try {
            if (!isoDep.isConnected) {
                isoDep.connect()
            }

            val response = isoDep.transceive(commandApdu)
            Log.d(tag, "APDU response: ${response.size} bytes")
            response
        } catch (e: Exception) {
            Log.e(tag, "APDU error", e)
            throw Exception("Tag connection lost, please hold your phone still")
        }
    }
}
