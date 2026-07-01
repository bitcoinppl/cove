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
import org.bitcoinppl.cove.R
import org.bitcoinppl.cove.UiText
import org.bitcoinppl.cove_core.*
import org.bitcoinppl.cove_core.TapcardTransportProtocol
import org.bitcoinppl.cove_core.TransportException
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
    private val mainHandler = android.os.Handler(android.os.Looper.getMainLooper())

    // current operation state
    private var currentCmd: TapSignerCmd? = null
    private var tagDetected = CompletableDeferred<Tag>()
    private var isScanning = false
    private var pendingDisableRunnable: Runnable? = null

    // message callback for UI updates (setMessage/appendMessage from transport)
    var onMessageUpdate: ((UiText) -> Unit)? = null

    // callback when NFC tag is first detected (before reading starts)
    var onTagDetected: (() -> Unit)? = null

    // prevents concurrent NFC operations
    private val operationMutex = Mutex()

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
            val activity = activityRef?.get() ?: throw Exception(appString(R.string.tap_signer_activity_unavailable))
            val nfcAdapter = this.nfcAdapter ?: throw Exception(appString(R.string.tap_signer_nfc_not_available))

            if (!nfcAdapter.isEnabled) {
                throw Exception(activity.getString(R.string.nfc_disabled))
            }

            Log.d(tag, "Starting NFC scan for command: $cmd")

            return@withLock suspendCancellableCoroutine { continuation ->
                // cancel any pending disable from previous operation
                pendingDisableRunnable?.let { mainHandler.removeCallbacks(it) }
                pendingDisableRunnable = null

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
                                onTagDetected?.invoke()
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
                        var reader: TapSignerReader? = null
                        try {
                            // wait for tag with timeout
                            val detectedTag =
                                withTimeoutOrNull(NFC_SCAN_TIMEOUT_MS) {
                                    tagDetected.await()
                                } ?: throw Exception(activity.getString(R.string.tap_signer_nfc_scan_timeout))

                            Log.d(tag, "Processing detected tag")

                            // get IsoDep tech (ISO7816)
                            isoDep =
                                IsoDep.get(detectedTag)
                                    ?: throw Exception(activity.getString(R.string.tap_signer_iso_dep_missing))

                            // connect to tag with increased timeout for reliability
                            if (!isoDep.isConnected) {
                                isoDep.connect()
                            }

                            // use higher timeout for backup and setup (both run backup APDUs)
                            val needsLongTimeout =
                                cmd is TapSignerCmd.Backup || cmd is TapSignerCmd.Setup
                            val timeout =
                                if (needsLongTimeout) {
                                    TapCardTransport.ISODEP_BACKUP_TIMEOUT_MS
                                } else {
                                    TapCardTransport.ISODEP_TIMEOUT_MS
                                }
                            isoDep.timeout = timeout

                            Log.d(
                                tag,
                                "Connected to IsoDep tag (timeout=${timeout}ms, cmd=${cmd::class.simpleName})",
                            )

                            // send proactive UX guidance for heavy NFC operations
                            if (needsLongTimeout) {
                                onMessageUpdate?.invoke(UiText.resource(R.string.tap_signer_keep_steady))
                            }

                            // create transport with message callback and appropriate timeout
                            val transport =
                                TapCardTransport(
                                    isoDep = isoDep,
                                    onMessageUpdate = onMessageUpdate,
                                    timeoutMs = timeout,
                                    connectionLostMessage = activity.getString(R.string.tap_signer_connection_lost),
                                )

                            // create TapSignerReader using factory function (workaround for UniFFI async constructor limitation)
                            Log.d(tag, "Creating TapSignerReader with command using factory function")
                            reader = createTapSignerReader(transport, cmd)

                            // run the reader and get response
                            Log.d(tag, "Running TapSignerReader")
                            val response = reader.run()

                            Log.d(tag, "TapSigner command completed successfully")

                            // extract result using successResult function
                            val result =
                                successResult(response)
                                    ?: throw Exception(activity.getString(R.string.tap_signer_result_extraction_failed))

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
                            reader?.close()
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
                // delay disabling reader mode to prevent system from picking up
                // the TapSigner's NDEF URL and opening browser
                pendingDisableRunnable =
                    Runnable {
                        nfcAdapter?.disableReaderMode(activity)
                        Log.d(tag, "Stopped NFC scanning")
                        pendingDisableRunnable = null
                    }
                mainHandler.postDelayed(pendingDisableRunnable!!, READER_MODE_DISABLE_DELAY_MS)
            }
        }
    }

    companion object {
        @Volatile
        private var instance: TapCardNfcManager? = null

        // timeout for NFC tag detection
        private const val NFC_SCAN_TIMEOUT_MS = 60_000L

        // delay before disabling reader mode to give user time to move card away
        private const val READER_MODE_DISABLE_DELAY_MS = 5000L

        fun getInstance(): TapCardNfcManager =
            instance ?: synchronized(this) {
                instance ?: TapCardNfcManager().also { instance = it }
            }
    }

    private fun appString(id: Int): String =
        activityRef?.get()?.getString(id) ?: ""
}

/**
 * Android NFC transport implementation
 * implements TapcardTransportProtocol for IsoDep tags
 */
private class TapCardTransport(
    private val isoDep: IsoDep,
    private val onMessageUpdate: ((UiText) -> Unit)?,
    private val timeoutMs: Int = ISODEP_TIMEOUT_MS,
    private val connectionLostMessage: String,
) : TapcardTransportProtocol {
    private val tag = "TapCardTransport"
    private var currentMessage = ""

    override fun setMessage(message: String) {
        Log.d(tag, "Message: $message")
        currentMessage = message
        onMessageUpdate?.invoke(currentMessage.toUiText())
    }

    override fun appendMessage(message: String) {
        Log.d(tag, "Append: $message")
        currentMessage += message
        onMessageUpdate?.invoke(currentMessage.toUiText())
    }

    override suspend fun transmitApdu(commandApdu: ByteArray): ByteArray {
        Log.d(tag, "Transmitting APDU: ${commandApdu.size} bytes")

        return try {
            if (!isoDep.isConnected) {
                isoDep.connect()
                isoDep.timeout = timeoutMs
            }

            val response = isoDep.transceive(commandApdu)
            Log.d(tag, "APDU response: ${response.size} bytes")
            response
        } catch (e: Exception) {
            Log.e(tag, "APDU error", e)
            throw TransportException.UnknownException(
                connectionLostMessage,
            )
        }
    }

    companion object {
        // IsoDep transceive timeout (default is ~2s which is too short for some operations)
        const val ISODEP_TIMEOUT_MS = 5000

        // higher timeout for backup operations (heavier multi-step APDU exchange)
        const val ISODEP_BACKUP_TIMEOUT_MS = 8000

        private val pinAttemptsWaitPattern = Regex("""Too many PIN attempts, waiting for (\d+) seconds\.\.\.""")
    }

    private fun String.toUiText(): UiText {
        val pinAttemptsWait = pinAttemptsWaitPattern.matchEntire(this)
        if (pinAttemptsWait != null) {
            val seconds = pinAttemptsWait.groupValues[1].toIntOrNull() ?: return UiText.resource(R.string.tap_signer_keep_steady)
            return UiText.resource(R.string.tap_signer_too_many_pin_attempts_waiting, seconds)
        }

        return UiText.resource(R.string.tap_signer_keep_steady)
    }
}
