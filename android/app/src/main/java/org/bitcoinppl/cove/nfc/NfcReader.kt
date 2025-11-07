package org.bitcoinppl.cove.nfc

import android.app.Activity
import android.nfc.NdefMessage
import android.nfc.NfcAdapter
import android.nfc.Tag
import android.nfc.tech.Ndef
import android.nfc.tech.NdefFormatable
import android.util.Log
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.receiveAsFlow
import java.nio.charset.Charset

class NfcReader(private val activity: Activity) {
    private val nfcAdapter: NfcAdapter? = NfcAdapter.getDefaultAdapter(activity)
    private val _scanResults = Channel<NfcScanResult>(Channel.BUFFERED)
    val scanResults: Flow<NfcScanResult> = _scanResults.receiveAsFlow()

    var isScanning by mutableStateOf(false)
        private set

    var message by mutableStateOf("")
        private set

    fun startScanning() {
        if (nfcAdapter == null) {
            _scanResults.trySend(NfcScanResult.Error("NFC is not supported on this device"))
            return
        }

        if (!nfcAdapter.isEnabled) {
            _scanResults.trySend(NfcScanResult.Error("NFC is disabled. Please enable it in Settings"))
            return
        }

        isScanning = true
        message = "Hold your phone near the NFC tag"

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

    fun stopScanning() {
        isScanning = false
        message = ""
        nfcAdapter?.disableReaderMode(activity)
    }

    private fun handleTag(tag: Tag) {
        Log.d("NfcReader", "Tag detected: ${tag.techList.joinToString()}")

        try {
            // try reading NDEF data
            val ndef = Ndef.get(tag)
            if (ndef != null) {
                ndef.connect()
                val ndefMessage = ndef.ndefMessage
                ndef.close()

                if (ndefMessage != null) {
                    processNdefMessage(ndefMessage)
                    return
                }
            }

            // if NDEF didn't work, try to format and read
            val ndefFormatable = NdefFormatable.get(tag)
            if (ndefFormatable != null) {
                _scanResults.trySend(NfcScanResult.Error("Tag is not formatted with NDEF data"))
                return
            }

            _scanResults.trySend(NfcScanResult.Error("Unable to read NFC tag"))
        } catch (e: Exception) {
            Log.e("NfcReader", "Error reading NFC tag", e)
            _scanResults.trySend(NfcScanResult.Error("Error reading tag: ${e.message}"))
        }
    }

    private fun processNdefMessage(ndefMessage: NdefMessage) {
        Log.d("NfcReader", "Processing NDEF message with ${ndefMessage.records.size} records")

        var textContent = ""
        var binaryData: ByteArray? = null

        for (record in ndefMessage.records) {
            Log.d("NfcReader", "Record type: ${String(record.type)}")

            // try to extract text
            val payload = record.payload
            if (payload.isNotEmpty()) {
                // check if it's a text record (TNF_WELL_KNOWN with type "T")
                if (record.tnf == android.nfc.NdefRecord.TNF_WELL_KNOWN &&
                    String(record.type) == "T"
                ) {
                    // text record format: first byte is status, rest is text
                    val statusByte = payload[0]
                    val textEncoding = if (statusByte.toInt() and 0x80 == 0) "UTF-8" else "UTF-16"
                    val languageCodeLength = statusByte.toInt() and 0x3F
                    val text =
                        String(
                            payload,
                            languageCodeLength + 1,
                            payload.size - languageCodeLength - 1,
                            Charset.forName(textEncoding),
                        )
                    textContent = text
                    Log.d("NfcReader", "Found text: $text")
                } else {
                    // try as raw string
                    try {
                        val text = String(payload, Charsets.UTF_8)
                        if (text.isNotBlank()) {
                            textContent = text
                            Log.d("NfcReader", "Found raw text: $text")
                        }
                    } catch (e: Exception) {
                        Log.d("NfcReader", "Not text, storing as binary")
                        binaryData = payload
                    }
                }
            }
        }

        if (textContent.isNotBlank()) {
            _scanResults.trySend(NfcScanResult.Success(textContent, binaryData))
        } else if (binaryData != null) {
            _scanResults.trySend(NfcScanResult.Success(null, binaryData))
        } else {
            _scanResults.trySend(NfcScanResult.Error("No readable data found on NFC tag"))
        }

        stopScanning()
    }

    fun reset() {
        stopScanning()
        isScanning = false
        message = ""
    }
}

sealed class NfcScanResult {
    data class Success(val text: String?, val data: ByteArray?) : NfcScanResult() {
        override fun equals(other: Any?): Boolean {
            if (this === other) return true
            if (javaClass != other?.javaClass) return false

            other as Success

            if (text != other.text) return false
            if (data != null) {
                if (other.data == null) return false
                if (!data.contentEquals(other.data)) return false
            } else if (other.data != null) {
                return false
            }

            return true
        }

        override fun hashCode(): Int {
            var result = text?.hashCode() ?: 0
            result = 31 * result + (data?.contentHashCode() ?: 0)
            return result
        }
    }

    data class Error(val message: String) : NfcScanResult()
}
