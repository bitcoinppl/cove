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
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.receiveAsFlow
import org.bitcoinppl.cove.Log
import org.bitcoinppl.cove.R
import org.bitcoinppl.cove.UiText

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

    var message by mutableStateOf<UiText?>(null)
        private set

    var writingState by mutableStateOf(NfcWritingState.WAITING)
        private set

    private var dataToWrite: ByteArray? = null

    private fun sendError(errorMessage: UiText) {
        mainHandler.post {
            writingState = NfcWritingState.WAITING
            message = UiText.resource(R.string.nfc_hold_near_tag)
        }
        _writeResults.trySend(NfcWriteResult.Error(errorMessage))
    }

    fun startWriting(data: ByteArray) {
        if (nfcAdapter == null) {
            _writeResults.trySend(NfcWriteResult.Error(UiText.resource(R.string.nfc_not_supported)))
            return
        }

        if (!nfcAdapter.isEnabled) {
            _writeResults.trySend(NfcWriteResult.Error(UiText.resource(R.string.nfc_disabled)))
            return
        }

        dataToWrite = data
        isWriting = true
        message = UiText.resource(R.string.nfc_hold_near_tag)
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
        message = null
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
            sendError(UiText.resource(R.string.nfc_no_data_to_write))
            return
        }

        // update state on main thread - tag detected
        mainHandler.post {
            writingState = NfcWritingState.TAG_DETECTED
            message = UiText.resource(R.string.nfc_tag_detected)
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

            sendError(UiText.resource(R.string.nfc_tag_does_not_support_ndef))
        } catch (e: Exception) {
            Log.e(TAG, "Error writing NFC tag", e)
            sendError(
                UiText.resource(R.string.nfc_error_writing_tag),
            )
        }
    }

    private fun writeToNdef(
        ndef: Ndef,
        message: NdefMessage,
    ) {
        mainHandler.post {
            writingState = NfcWritingState.WRITING
            this.message = UiText.resource(R.string.nfc_writing_hold_still)
        }

        try {
            ndef.connect()

            if (!ndef.isWritable) {
                ndef.close()
                sendError(UiText.resource(R.string.nfc_tag_not_writable))
                return
            }

            val messageSize = message.toByteArray().size
            if (messageSize > ndef.maxSize) {
                ndef.close()
                sendError(UiText.resource(R.string.nfc_data_too_large, messageSize, ndef.maxSize))
                return
            }

            ndef.writeNdefMessage(message)
            ndef.close()

            Log.d(TAG, "Successfully wrote $messageSize bytes to NFC tag")

            // success
            mainHandler.post {
                writingState = NfcWritingState.SUCCESS
                this.message = UiText.resource(R.string.nfc_tag_written_successfully)
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
            sendError(
                UiText.resource(R.string.nfc_write_failed),
            )
        }
    }

    private fun formatAndWrite(
        ndefFormatable: NdefFormatable,
        message: NdefMessage,
    ) {
        mainHandler.post {
            writingState = NfcWritingState.WRITING
            this.message = UiText.resource(R.string.nfc_formatting_and_writing)
        }

        try {
            ndefFormatable.connect()
            ndefFormatable.format(message)
            ndefFormatable.close()

            Log.d(TAG, "Successfully formatted and wrote to NFC tag")

            // success
            mainHandler.post {
                writingState = NfcWritingState.SUCCESS
                this.message = UiText.resource(R.string.nfc_tag_written_successfully)
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
            sendError(
                UiText.resource(R.string.nfc_format_failed),
            )
        }
    }

    fun reset() {
        stopWriting()
        isWriting = false
        message = null
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
        val message: UiText,
    ) : NfcWriteResult()
}
