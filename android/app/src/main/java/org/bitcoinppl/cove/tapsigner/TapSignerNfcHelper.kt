package org.bitcoinppl.cove.tapsigner

import android.app.Activity
import android.nfc.NfcAdapter
import android.nfc.tech.IsoDep
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.suspendCancellableCoroutine
import kotlinx.coroutines.withTimeout
import org.bitcoinppl.cove_core.*
import org.bitcoinppl.cove_core.TapcardTransportProtocol
import org.bitcoinppl.cove_core.tapcard.TapSigner
import org.bitcoinppl.cove_core.types.Psbt
import java.util.concurrent.atomic.AtomicBoolean
import kotlin.coroutines.resume
import kotlin.coroutines.resumeWithException

// helper to create TapSignerReader since async constructor isn't generated for Kotlin
private suspend fun createTapSignerReader(
    transport: TapcardTransportProtocol,
    cmd: TapSignerCmd?,
): TapSignerReader {
    return uniffiRustCallAsync(
        UniffiLib.uniffi_cove_fn_constructor_tapsignerreader_new(
            FfiConverterTypeTapcardTransportProtocol.lower(transport),
            FfiConverterOptionalTypeTapSignerCmd.lower(cmd),
        ),
        { future, callback, continuation ->
            UniffiLib.ffi_cove_rust_future_poll_u64(future, callback, continuation)
        },
        { future, continuation ->
            UniffiLib.ffi_cove_rust_future_complete_u64(future, continuation)
        },
        { future ->
            UniffiLib.ffi_cove_rust_future_free_u64(future)
        },
        { FfiConverterTypeTapSignerReader.lift(it) },
        TapSignerReaderException.ErrorHandler,
    )
}

/**
 * NFC helper for TapSigner operations
 * ported from iOS TapSignerNFC.swift
 */
class TapSignerNfcHelper(
    private val tapSigner: TapSigner,
) {
    private val tag = "TapSignerNfcHelper"
    private var lastResponse: TapSignerResponse? = null

    suspend fun setupTapSigner(
        activity: Activity,
        factoryPin: String,
        newPin: String,
        chainCode: ByteArray? = null,
    ): SetupCmdResponse {
        return try {
            doSetupTapSigner(activity, factoryPin, newPin, chainCode)
        } catch (e: Exception) {
            android.util.Log.e(tag, "Setup failed", e)
            throw e
        }
    }

    suspend fun derive(activity: Activity, pin: String): DeriveInfo {
        return performTapSignerCmd(activity, TapSignerCmd.Derive(pin)) { response ->
            (response as? TapSignerResponse.Import)?.v1
        }
    }

    suspend fun changePin(
        activity: Activity,
        currentPin: String,
        newPin: String,
    ) {
        performTapSignerCmd<Unit>(activity, TapSignerCmd.Change(currentPin, newPin)) { response ->
            if (response is TapSignerResponse.Change) Unit else null
        }
    }

    suspend fun backup(activity: Activity, pin: String): ByteArray {
        return performTapSignerCmd(activity, TapSignerCmd.Backup(pin)) { response ->
            (response as? TapSignerResponse.Backup)?.v1
        }
    }

    suspend fun sign(
        activity: Activity,
        psbt: Psbt,
        pin: String,
    ): Psbt {
        return performTapSignerCmd(activity, TapSignerCmd.Sign(psbt, pin)) { response ->
            (response as? TapSignerResponse.Sign)?.v1
        }
    }

    fun lastResponse(): TapSignerResponse? = lastResponse

    private suspend fun <T> performTapSignerCmd(
        activity: Activity,
        cmd: TapSignerCmd,
        successResult: (TapSignerResponse?) -> T?,
    ): T =
        withTimeout(90_000) {
            suspendCancellableCoroutine { continuation ->
                val nfcAdapter = NfcAdapter.getDefaultAdapter(activity)

                if (nfcAdapter == null) {
                    continuation.resumeWithException(Exception("NFC is not supported on this device"))
                    return@suspendCancellableCoroutine
                }

                if (!nfcAdapter.isEnabled) {
                    continuation.resumeWithException(Exception("NFC is disabled. Please enable it in Settings"))
                    return@suspendCancellableCoroutine
                }

                val hasResumed = AtomicBoolean(false)

                // enable reader mode for ISO14443 tags (TapSigner uses ISO7816)
                nfcAdapter.enableReaderMode(
                    activity,
                    { nfcTag ->
                        if (hasResumed.get()) return@enableReaderMode

                        // launch coroutine to handle async operations
                        CoroutineScope(Dispatchers.IO).launch {
                            try {
                                // get IsoDep technology
                                val isoDep = IsoDep.get(nfcTag)
                                if (isoDep == null) {
                                    if (hasResumed.compareAndSet(false, true)) {
                                        nfcAdapter.disableReaderMode(activity)
                                        continuation.resumeWithException(Exception("Tag does not support IsoDep"))
                                    }
                                    return@launch
                                }

                                // connect to tag
                                isoDep.connect()
                                android.util.Log.d(tag, "Connected to IsoDep tag")

                                // create transport and reader
                                val transport = TapCardTransport(isoDep)
                                val reader = createTapSignerReader(transport, cmd)

                                // run the command
                                val response = reader.run()
                                lastResponse = response

                                // extract result
                                val result = successResult(response)
                                if (result != null) {
                                    if (hasResumed.compareAndSet(false, true)) {
                                        nfcAdapter.disableReaderMode(activity)
                                        continuation.resume(result)
                                    }
                                } else {
                                    if (hasResumed.compareAndSet(false, true)) {
                                        nfcAdapter.disableReaderMode(activity)
                                        continuation.resumeWithException(
                                            Exception("Failed to extract result from response"),
                                        )
                                    }
                                }

                                // cleanup
                                try {
                                    if (isoDep.isConnected) {
                                        isoDep.close()
                                    }
                                } catch (e: Exception) {
                                    android.util.Log.e(tag, "Error closing IsoDep", e)
                                }
                            } catch (e: Exception) {
                                android.util.Log.e(tag, "Error processing TapSigner command", e)
                                if (hasResumed.compareAndSet(false, true)) {
                                    nfcAdapter.disableReaderMode(activity)
                                    continuation.resumeWithException(e)
                                }
                            }
                        }
                    },
                    NfcAdapter.FLAG_READER_NFC_A or
                        NfcAdapter.FLAG_READER_SKIP_NDEF_CHECK or
                        NfcAdapter.FLAG_READER_NO_PLATFORM_SOUNDS,
                    null,
                )

                // cleanup when cancelled
                continuation.invokeOnCancellation {
                    nfcAdapter.disableReaderMode(activity)
                }
            }
        }

    private suspend fun doSetupTapSigner(
        activity: Activity,
        factoryPin: String,
        newPin: String,
        chainCode: ByteArray?,
    ): SetupCmdResponse {
        val cmd = SetupCmd.tryNew(factoryPin, newPin, chainCode)
        return performTapSignerCmd(activity, TapSignerCmd.Setup(cmd)) { response ->
            (response as? TapSignerResponse.Setup)?.v1
        }
    }

    suspend fun continueSetup(activity: Activity, response: SetupCmdResponse): SetupCmdResponse {
        val cmd =
            when (response) {
                is SetupCmdResponse.ContinueFromInit -> response.v1.continueCmd
                is SetupCmdResponse.ContinueFromBackup -> response.v1.continueCmd
                is SetupCmdResponse.ContinueFromDerive -> response.v1.continueCmd
                is SetupCmdResponse.Complete -> null
            }

        if (cmd == null) return response

        return performTapSignerCmd(activity, TapSignerCmd.Setup(cmd)) { resp ->
            (resp as? TapSignerResponse.Setup)?.v1
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
    override fun setMessage(message: String) {
        // Android NFC doesn't support updating UI message during transaction
        android.util.Log.d("TapCardTransport", "Message: $message")
    }

    override fun appendMessage(message: String) {
        android.util.Log.d("TapCardTransport", "Append: $message")
    }

    override suspend fun transmitApdu(commandApdu: ByteArray): ByteArray {
        android.util.Log.d("TapCardTransport", "Transmitting APDU: ${commandApdu.size} bytes")

        return try {
            if (!isoDep.isConnected) {
                isoDep.connect()
            }

            val response = isoDep.transceive(commandApdu)
            android.util.Log.d("TapCardTransport", "APDU response: ${response.size} bytes")
            response
        } catch (e: Exception) {
            android.util.Log.e("TapCardTransport", "APDU error", e)
            throw e
        }
    }
}
