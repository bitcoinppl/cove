package org.bitcoinppl.cove.nfc

import android.app.Activity
import android.nfc.NdefMessage
import android.nfc.NdefRecord
import android.nfc.NfcAdapter
import android.nfc.Tag
import android.nfc.tech.Ndef
import android.nfc.tech.NdefFormatable
import android.os.Handler
import android.os.Looper
import android.util.Log
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.receiveAsFlow

enum class NfcWritingState {
    WAITING,
    TAG_DETECTED,
    WRITING,
    SUCCESS,
}

class NfcWriter(
    private val activity: Activity,
) {
    private val nfcAdapter: NfcAdapter? = NfcAdapter.getDefaultAdapter(activity)
    private val _writeResults = Channel<NfcWriteResult>(Channel.BUFFERED)
    val writeResults: Flow<NfcWriteResult> = _writeResults.receiveAsFlow()
    private val mainHandler = Handler(Looper.getMainLooper())

    var isWriting by mutableStateOf(false)
        private set

    var message by mutableStateOf("")
        private set

    var writingState by mutableStateOf(NfcWritingState.WAITING)
        private set

    private var dataToWrite: ByteArray? = null

    fun startWriting(data: ByteArray) {
        if (nfcAdapter == null) {
            _writeResults.trySend(NfcWriteResult.Error("NFC is not supported on this device"))
            return
        }

        if (!nfcAdapter.isEnabled) {
            _writeResults.trySend(NfcWriteResult.Error("NFC is disabled. Please enable it in Settings"))
            return
        }

        dataToWrite = data
        isWriting = true
        message = "Hold your phone near the NFC tag"
        writingState = NfcWritingState.WAITING

        // must be called on UI thread
        activity.runOnUiThread {
            nfcAdapter.enableReaderMode(
                activity,
                { tag ->
                    handleTag(tag)
                },
                NfcAdapter.FLAG_READER_NFC_A or
                    NfcAdapter.FLAG_READER_NFC_B or
                    NfcAdapter.FLAG_READER_NFC_F or
                    NfcAdapter.FLAG_READER_NFC_V or
                    NfcAdapter.FLAG_READER_NO_PLATFORM_SOUNDS,
                null,
            )
        }
    }

    fun stopWriting() {
        // clear any pending delayed callbacks first
        mainHandler.removeCallbacksAndMessages(null)
        isWriting = false
        message = ""
        writingState = NfcWritingState.WAITING
        dataToWrite = null
        // must be called on UI thread
        activity.runOnUiThread {
            nfcAdapter?.disableReaderMode(activity)
        }
    }

    private fun handleTag(tag: Tag) {
        Log.d(TAG, "Tag detected: ${tag.techList.joinToString()}")

        val data = dataToWrite
        if (data == null) {
            _writeResults.trySend(NfcWriteResult.Error("No data to write"))
            return
        }

        // update state on main thread - tag detected
        mainHandler.post {
            writingState = NfcWritingState.TAG_DETECTED
            message = "Tag detected"
        }

        try {
            // create NDEF message with binary payload
            val record = NdefRecord.createMime("application/octet-stream", data)
            val ndefMessage = NdefMessage(arrayOf(record))

            // try writing to NDEF tag
            val ndef = Ndef.get(tag)
            if (ndef != null) {
                writeToNdef(ndef, ndefMessage)
                return
            }

            // try formatting and writing
            val ndefFormatable = NdefFormatable.get(tag)
            if (ndefFormatable != null) {
                formatAndWrite(ndefFormatable, ndefMessage)
                return
            }

            _writeResults.trySend(NfcWriteResult.Error("Tag doesn't support NDEF"))
        } catch (e: Exception) {
            Log.e(TAG, "Error writing NFC tag", e)
            _writeResults.trySend(NfcWriteResult.Error("Error writing tag: ${e.message}"))
        }
    }

    private fun writeToNdef(
        ndef: Ndef,
        message: NdefMessage,
    ) {
        mainHandler.post {
            writingState = NfcWritingState.WRITING
            this.message = "Writing, please hold still..."
        }

        try {
            ndef.connect()

            if (!ndef.isWritable) {
                ndef.close()
                _writeResults.trySend(NfcWriteResult.Error("Tag is not writable"))
                return
            }

            val messageSize = message.toByteArray().size
            if (messageSize > ndef.maxSize) {
                ndef.close()
                _writeResults.trySend(
                    NfcWriteResult.Error(
                        "Data too large for tag ($messageSize bytes, max ${ndef.maxSize})",
                    ),
                )
                return
            }

            ndef.writeNdefMessage(message)
            ndef.close()

            Log.d(TAG, "Successfully wrote $messageSize bytes to NFC tag")

            // success
            mainHandler.post {
                writingState = NfcWritingState.SUCCESS
                this.message = "Tag written successfully!"
            }

            // delay sending success result so UI can show success message
            mainHandler.postDelayed({
                _writeResults.trySend(NfcWriteResult.Success)
                stopWriting()
            }, SUCCESS_DISPLAY_DELAY_MS)
        } catch (e: Exception) {
            Log.e(TAG, "Error writing to NDEF tag", e)
            try {
                ndef.close()
            } catch (closeError: Exception) {
                // ignore close errors
            }
            _writeResults.trySend(NfcWriteResult.Error("Write failed: ${e.message}"))
        }
    }

    private fun formatAndWrite(
        ndefFormatable: NdefFormatable,
        message: NdefMessage,
    ) {
        mainHandler.post {
            writingState = NfcWritingState.WRITING
            this.message = "Formatting and writing..."
        }

        try {
            ndefFormatable.connect()
            ndefFormatable.format(message)
            ndefFormatable.close()

            Log.d(TAG, "Successfully formatted and wrote to NFC tag")

            // success
            mainHandler.post {
                writingState = NfcWritingState.SUCCESS
                this.message = "Tag written successfully!"
            }

            // delay sending success result so UI can show success message
            mainHandler.postDelayed({
                _writeResults.trySend(NfcWriteResult.Success)
                stopWriting()
            }, SUCCESS_DISPLAY_DELAY_MS)
        } catch (e: Exception) {
            Log.e(TAG, "Error formatting NFC tag", e)
            try {
                ndefFormatable.close()
            } catch (closeError: Exception) {
                // ignore close errors
            }
            _writeResults.trySend(NfcWriteResult.Error("Format failed: ${e.message}"))
        }
    }

    fun reset() {
        stopWriting()
        isWriting = false
        message = ""
        writingState = NfcWritingState.WAITING
        dataToWrite = null
    }

    fun close() {
        stopWriting()
        mainHandler.removeCallbacksAndMessages(null)
        _writeResults.close()
    }

    companion object {
        private const val TAG = "NfcWriter"
        private const val SUCCESS_DISPLAY_DELAY_MS = 1000L
    }
}

sealed class NfcWriteResult {
    data object Success : NfcWriteResult()

    data class Error(
        val message: String,
    ) : NfcWriteResult()
}
