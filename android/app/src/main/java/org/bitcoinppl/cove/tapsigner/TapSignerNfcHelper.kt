package org.bitcoinppl.cove.tapsigner

import android.nfc.NfcAdapter
import android.nfc.Tag
import android.nfc.tech.IsoDep
import kotlinx.coroutines.suspendCancellableCoroutine
import org.bitcoinppl.cove_core.*
import org.bitcoinppl.cove_core.TapcardTransportProtocol
import org.bitcoinppl.cove_core.tapcard.TapSigner
import org.bitcoinppl.cove_core.types.Psbt
import kotlin.coroutines.resume
import kotlin.coroutines.resumeWithException

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
        factoryPin: String,
        newPin: String,
        chainCode: ByteArray? = null,
    ): SetupCmdResponse {
        return try {
            doSetupTapSigner(factoryPin, newPin, chainCode)
        } catch (e: Exception) {
            android.util.Log.e(tag, "Setup failed", e)
            throw e
        }
    }

    suspend fun derive(pin: String): DeriveInfo {
        // placeholder implementation - actual NFC not yet implemented
        throw Exception("NFC operations not yet implemented for Android")
    }

    suspend fun changePin(
        currentPin: String,
        newPin: String,
    ) {
        performTapSignerCmd<Unit>(TapSignerCmd.Change(currentPin, newPin)) { response ->
            if (response is TapSignerResponse.Change) Unit else null
        }
    }

    suspend fun backup(pin: String): ByteArray {
        return performTapSignerCmd(TapSignerCmd.Backup(pin)) { response ->
            (response as? TapSignerResponse.Backup)?.v1
        }
    }

    suspend fun sign(
        psbt: Psbt,
        pin: String,
    ): Psbt {
        return performTapSignerCmd(TapSignerCmd.Sign(psbt, pin)) { response ->
            (response as? TapSignerResponse.Sign)?.v1
        }
    }

    fun lastResponse(): TapSignerResponse? = lastResponse

    private suspend fun <T> performTapSignerCmd(
        cmd: TapSignerCmd,
        successResult: (TapSignerResponse?) -> T?,
    ): T =
        suspendCancellableCoroutine { continuation ->
            // request NFC adapter to scan
            // this would integrate with Android's NFC system
            // for now, we'll throw an error indicating NFC not implemented
            continuation.resumeWithException(
                Exception("NFC operations not yet implemented for Android"),
            )

            // TODO: implement NFC scanning and tag detection
            // the actual implementation would:
            // 1. use NfcAdapter to enable reader mode
            // 2. detect ISO7816 tags
            // 3. create TapCardTransport from IsoDep
            // 4. create TapSignerReader with transport and cmd
            // 5. run the reader and get response
            // 6. call successResult with response
            // 7. handle errors appropriately
        }

    private suspend fun doSetupTapSigner(
        factoryPin: String,
        newPin: String,
        chainCode: ByteArray?,
    ): SetupCmdResponse {
        var errorCount = 0
        var lastError: Exception? = null

        return suspendCancellableCoroutine { continuation ->
            // TODO: implement setup flow similar to iOS
            // the flow would:
            // 1. create SetupCmd
            // 2. start NFC scanning
            // 3. create reader and run command
            // 4. handle incomplete responses with continueSetup
            // 5. retry on errors up to 5 times
            // 6. resume continuation with result

            continuation.resumeWithException(
                Exception("NFC operations not yet implemented for Android"),
            )
        }
    }

    suspend fun continueSetup(response: SetupCmdResponse): SetupCmdResponse {
        val cmd =
            when (response) {
                is SetupCmdResponse.ContinueFromInit -> response.v1.continueCmd
                is SetupCmdResponse.ContinueFromBackup -> response.v1.continueCmd
                is SetupCmdResponse.ContinueFromDerive -> response.v1.continueCmd
                is SetupCmdResponse.Complete -> null
            }

        if (cmd == null) return response

        return suspendCancellableCoroutine { continuation ->
            // TODO: implement continue setup
            continuation.resumeWithException(
                Exception("NFC operations not yet implemented for Android"),
            )
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
