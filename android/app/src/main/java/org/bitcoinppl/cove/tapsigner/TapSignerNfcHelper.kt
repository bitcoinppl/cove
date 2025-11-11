package org.bitcoinppl.cove.tapsigner

import android.util.Log
import kotlinx.coroutines.suspendCancellableCoroutine
import org.bitcoinppl.cove.nfc.TapCardNfcManager
import org.bitcoinppl.cove_core.*
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
    private val nfcManager = TapCardNfcManager.getInstance()
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
            Log.e(tag, "Setup failed", e)
            throw e
        }
    }

    suspend fun derive(pin: String): DeriveInfo {
        return performTapSignerCmd(TapSignerCmd.Derive(pin)) { response ->
            // NOTE: Derive returns Import response in Rust (see tap_signer_reader.rs)
            when (response) {
                is TapSignerResponse.Import -> response.v1
                else -> null
            }
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
    ): T {
        try {
            val result = nfcManager.performTapSignerCmd(cmd, successResult)
            // store last response for retry scenarios
            lastResponse = null // response is already extracted
            return result
        } catch (e: Exception) {
            Log.e(tag, "TapSigner command failed", e)
            throw e
        }
    }

        if (!nfcAdapter.isEnabled) {
            throw Exception("NFC is disabled. Please enable it in Settings")
        }

        return try {
            withTimeout(90_000) {
                suspendCancellableCoroutine { continuation ->
                    val hasResumed = AtomicBoolean(false)

                    // enable reader mode for ISO14443 tags (TapSigner uses ISO7816)
                    nfcAdapter.enableReaderMode(
                        activity,
                        { nfcTag ->
                            if (hasResumed.get()) return@enableReaderMode

                            // launch coroutine to handle async operations
                            CoroutineScope(Dispatchers.IO).launch {
                                var isoDep: IsoDep? = null
                                try {
                                    // get IsoDep technology
                                    isoDep = IsoDep.get(nfcTag)
                                    if (isoDep == null) {
                                        if (hasResumed.compareAndSet(false, true)) {
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
                                            continuation.resume(result)
                                        }
                                    } else {
                                        if (hasResumed.compareAndSet(false, true)) {
                                            continuation.resumeWithException(
                                                Exception("Failed to extract result from response"),
                                            )
                                        }
                                    }
                                } catch (e: Exception) {
                                    android.util.Log.e(tag, "Error processing TapSigner command", e)
                                    if (hasResumed.compareAndSet(false, true)) {
                                        continuation.resumeWithException(e)
                                    }
                                } finally {
                                    // always close IsoDep connection
                                    isoDep?.let {
                                        try {
                                            if (it.isConnected) {
                                                it.close()
                                            }
                                        } catch (e: Exception) {
                                            android.util.Log.e(tag, "Error closing IsoDep", e)
                                        }
                                    }
                                    // always disable reader mode after operation completes
                                    if (hasResumed.get()) {
                                        activity.runOnUiThread {
                                            nfcAdapter.disableReaderMode(activity)
                                        }
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
                        activity.runOnUiThread {
                            nfcAdapter.disableReaderMode(activity)
                        }
                    }
                }
            }
        } finally {
            activity.runOnUiThread {
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

        return performTapSignerCmd(TapSignerCmd.Setup(cmd)) { resp ->
            (resp as? TapSignerResponse.Setup)?.v1
        }
    }
}
