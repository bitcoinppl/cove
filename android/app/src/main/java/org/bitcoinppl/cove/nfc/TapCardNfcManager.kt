package org.bitcoinppl.cove.nfc

import android.app.Activity
import android.nfc.NfcAdapter
import android.nfc.Tag
import android.nfc.tech.IsoDep
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.suspendCancellableCoroutine
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.coroutines.withTimeoutOrNull
import org.bitcoinppl.cove.Log
import org.bitcoinppl.cove_core.*
import org.bitcoinppl.cove_core.TapcardTransportProtocol
import org.bitcoinppl.cove_core.createTapSignerReader
import java.lang.ref.WeakReference
import kotlin.coroutines.resume
import kotlin.coroutines.resumeWithException

/**
 * Manages NFC operations for TapCard (TapSigner/SatsCard)
 * singleton that's initialized with Activity context from MainActivity
 */
class TapCardNfcManager private constructor() {
    private val tag = "TapCardNfcManager"
    private var activityRef: WeakReference<Activity>? = null
    private var nfcAdapter: NfcAdapter? = null

    // current operation state
    private var currentCmd: TapSignerCmd? = null
    private var tagDetected = CompletableDeferred<Tag>()
    private var isScanning = false

    // message callback for UI updates (setMessage/appendMessage from transport)
    var onMessageUpdate: ((String) -> Unit)? = null

    // prevents concurrent NFC operations
    private val operationMutex = Mutex()

    companion object {
        @Volatile
        private var instance: TapCardNfcManager? = null

        // timeout for NFC tag detection - long enough for user to position phone
        // but not so long that they wonder if something is wrong
        private const val NFC_SCAN_TIMEOUT_MS = 60_000L

        fun getInstance(): TapCardNfcManager =
            instance ?: synchronized(this) {
                instance ?: TapCardNfcManager().also { instance = it }
            }
    }

    fun initialize(activity: Activity) {
        this.activityRef = WeakReference(activity)
        this.nfcAdapter = NfcAdapter.getDefaultAdapter(activity)
        Log.d(tag, "TapCardNfcManager initialized")
    }

    /**
     * Perform a TapSigner command by scanning for NFC tag
     * Returns a Pair of (extracted result, raw response) to enable retry scenarios
     */
    suspend fun <T> performTapSignerCmd(
        cmd: TapSignerCmd,
        successResult: (TapSignerResponse?) -> T?,
    ): Pair<T, TapSignerResponse?> =
        operationMutex.withLock {
            val activity = activityRef?.get() ?: throw Exception("Activity no longer available")
            val nfcAdapter = this.nfcAdapter ?: throw Exception("NFC not available on this device")

            if (!nfcAdapter.isEnabled) {
                throw Exception("NFC is disabled. Please enable it in Settings")
            }

            Log.d(tag, "Starting NFC scan for command: $cmd")

            return@withLock suspendCancellableCoroutine { continuation ->
                // reset state for new operation
                currentCmd = cmd
                tagDetected = CompletableDeferred()
                isScanning = true

                // enable reader mode for ISO14443 tags (TapSigner uses ISO7816)
                // must be called on UI thread
                activity.runOnUiThread {
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
                    Log.d(tag, "NFC reader mode enabled")
                }

                // handle tag detection and command execution
                val job =
                    CoroutineScope(Dispatchers.IO).launch {
                        var isoDep: IsoDep? = null
                        try {
                            // wait for tag with timeout
                            val detectedTag =
                                withTimeoutOrNull(NFC_SCAN_TIMEOUT_MS) {
                                    tagDetected.await()
                                } ?: throw Exception("NFC scan timed out. Please try again")

                            Log.d(tag, "Processing detected tag")

                            // get IsoDep tech (ISO7816)
                            isoDep =
                                IsoDep.get(detectedTag)
                                    ?: throw Exception("Tag doesn't support IsoDep (ISO7816)")

                            // connect to tag
                            if (!isoDep.isConnected) {
                                isoDep.connect()
                            }

                            Log.d(tag, "Connected to IsoDep tag")

                            // create transport with message callback
                            val transport = TapCardTransport(isoDep, onMessageUpdate)

                            // create TapSignerReader using factory function (workaround for UniFFI async constructor limitation)
                            Log.d(tag, "Creating TapSignerReader with command using factory function")
                            val reader = createTapSignerReader(transport, cmd)

                            // run the reader and get response
                            Log.d(tag, "Running TapSignerReader")
                            val response = reader.run()

                            Log.d(tag, "TapSigner command completed successfully")

                            // extract result using successResult function
                            val result =
                                successResult(response)
                                    ?: throw Exception("Command completed but result extraction failed")

                            // return both extracted result and raw response for retry scenarios
                            val resultPair = Pair(result, response)

                            // guard against cancelled continuation
                            if (continuation.isActive) {
                                continuation.resume(resultPair)
                            }
                        } catch (e: TapSignerReaderException) {
                            Log.e(tag, "TapSigner error", e)

                            // guard against cancelled continuation
                            if (continuation.isActive) {
                                continuation.resumeWithException(e)
                            }
                        } catch (e: Exception) {
                            Log.e(tag, "NFC operation failed", e)
                            // guard against cancelled continuation
                            if (continuation.isActive) {
                                continuation.resumeWithException(e)
                            }
                        } finally {
                            // always clean up NFC resources
                            stopScanning()
                            isoDep?.let {
                                runCatching {
                                    if (it.isConnected) {
                                        it.close()
                                        Log.d(tag, "IsoDep connection closed")
                                    }
                                }
                            }
                        }
                    }

                // handle cancellation
                continuation.invokeOnCancellation {
                    Log.d(tag, "NFC operation cancelled")
                    job.cancel()
                    stopScanning()
                }
            }
        }

    private fun stopScanning() {
        if (isScanning) {
            isScanning = false
            val activity = activityRef?.get()
            if (activity != null) {
                activity.runOnUiThread {
                    nfcAdapter?.disableReaderMode(activity)
                    Log.d(tag, "Stopped NFC scanning")
                }
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
    private val onMessageUpdate: ((String) -> Unit)?,
) : TapcardTransportProtocol {
    private val tag = "TapCardTransport"
    private var currentMessage = ""

    override fun setMessage(message: String) {
        Log.d(tag, "Message: $message")
        currentMessage = message
        onMessageUpdate?.invoke(currentMessage)
    }

    override fun appendMessage(message: String) {
        Log.d(tag, "Append: $message")
        currentMessage += message
        onMessageUpdate?.invoke(currentMessage)
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
