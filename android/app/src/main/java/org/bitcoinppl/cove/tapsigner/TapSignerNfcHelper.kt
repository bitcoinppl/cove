package org.bitcoinppl.cove.tapsigner

import android.app.Activity
import android.nfc.NfcAdapter
import android.nfc.Tag
import android.nfc.tech.IsoDep
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.suspendCancellableCoroutine
import org.bitcoinppl.cove_core.*
import org.bitcoinppl.cove_core.TapcardTransportProtocol
import org.bitcoinppl.cove_core.tapcard.TapSigner
import org.bitcoinppl.cove_core.types.Psbt
import kotlin.coroutines.resume
import kotlin.coroutines.resumeWithException

/**
 * Android NFC implementation for TapSigner hardware wallet operations.
 *
 * This class provides platform-specific NFC integration using Android's IsoDep API
 * to communicate with TapSigner cards. It wraps low-level NFC operations and delegates
 * business logic to Rust via UniFFI-generated TapSignerReader.
 *
 * Architecture:
 * - UI Layer (Compose) → TapSignerManager → TapSignerNfcHelper → Rust FFI → Hardware
 * - Uses NfcAdapter reader mode for tag detection
 * - Creates TapCardTransport implementing TapcardTransportProtocol for APDU transmission
 * - Rust TapSignerReader handles protocol complexity and multi-step flows
 *
 * Lifecycle:
 * - Requires Activity context for NFC reader mode enable/disable
 * - Reader mode is enabled per-operation and disabled after completion
 * - Operations are suspended functions that resume when NFC completes
 *
 * Reference: iOS implementation in ios/Cove/TapSignerNFC.swift
 *
 * @property activity Activity context required for NFC adapter operations
 * @property tapSigner TapSigner instance from Rust containing card state
 */
class TapSignerNfcHelper(
    private val activity: Activity,
    private val tapSigner: TapSigner,
) {
    private val tag = "TapSignerNfcHelper"
    private var lastResponse: TapSignerResponse? = null
    private val nfcAdapter: NfcAdapter? = NfcAdapter.getDefaultAdapter(activity)

    /**
     * Performs initial TapSigner setup with factory PIN.
     *
     * This is a multi-step process that:
     * 1. Verifies factory PIN and creates new PIN
     * 2. Performs backup operation
     * 3. Derives master extended public key
     * 4. Completes PIN change
     *
     * Includes retry logic (up to 5 attempts) for handling transient NFC errors.
     *
     * @param factoryPin Factory default PIN (usually "000000")
     * @param newPin New PIN to set (must meet TapSigner requirements)
     * @param chainCode Optional custom chain code for key derivation
     * @return SetupCmdResponse containing derived key info on success
     * @throws Exception if setup fails after retries or if NFC is unavailable
     */
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

    /**
     * Derives extended public key from TapSigner.
     *
     * Retrieves the master extended public key (xpub) from the card using the provided PIN.
     * This is typically used when importing an existing TapSigner wallet.
     *
     * @param pin TapSigner PIN for authentication
     * @return DeriveInfo containing the extended public key and derivation path
     * @throws Exception if PIN is incorrect or NFC communication fails
     */
    suspend fun derive(pin: String): DeriveInfo {
        return performTapSignerCmd(TapSignerCmd.Derive(pin)) { response ->
            (response as? TapSignerResponse.Import)?.v1
        }
    }

    /**
     * Changes the TapSigner PIN.
     *
     * @param currentPin Current PIN for authentication
     * @param newPin New PIN to set (must meet TapSigner requirements)
     * @throws Exception if current PIN is incorrect or NFC communication fails
     */
    suspend fun changePin(
        currentPin: String,
        newPin: String,
    ) {
        performTapSignerCmd<Unit>(TapSignerCmd.Change(currentPin, newPin)) { response ->
            if (response is TapSignerResponse.Change) Unit else null
        }
    }

    /**
     * Backs up the TapSigner master private key.
     *
     * Retrieves the encrypted master private key from the card for backup purposes.
     * The backup can be used to restore access if the card is lost or damaged.
     *
     * @param pin TapSigner PIN for authentication
     * @return Encrypted master private key bytes
     * @throws Exception if PIN is incorrect or NFC communication fails
     */
    suspend fun backup(pin: String): ByteArray {
        return performTapSignerCmd(TapSignerCmd.Backup(pin)) { response ->
            (response as? TapSignerResponse.Backup)?.v1
        }
    }

    /**
     * Signs a partially signed bitcoin transaction (PSBT) using the TapSigner.
     *
     * @param psbt PSBT to sign
     * @param pin TapSigner PIN for authentication
     * @return Signed PSBT with TapSigner's signature added
     * @throws Exception if PIN is incorrect or NFC communication fails
     */
    suspend fun sign(
        psbt: Psbt,
        pin: String,
    ): Psbt {
        return performTapSignerCmd(TapSignerCmd.Sign(psbt, pin)) { response ->
            (response as? TapSignerResponse.Sign)?.v1
        }
    }

    /**
     * Returns the last TapSigner response received.
     *
     * Useful for debugging and displaying detailed card state information.
     *
     * @return Last TapSignerResponse or null if no operations have been performed
     */
    fun lastResponse(): TapSignerResponse? = lastResponse

    /**
     * Generic command executor that handles NFC reader mode lifecycle.
     *
     * This suspends until an NFC tag is detected, executes the command, and returns the result.
     * Reader mode is automatically enabled before the operation and disabled after completion.
     *
     * @param T Expected result type
     * @param cmd TapSigner command to execute
     * @param successResult Lambda to extract typed result from TapSignerResponse
     * @return Command result of type T
     * @throws Exception if NFC is unavailable, disabled, or command fails
     */
    private suspend fun <T> performTapSignerCmd(
        cmd: TapSignerCmd,
        successResult: (TapSignerResponse?) -> T?,
    ): T =
        suspendCancellableCoroutine { continuation ->
            if (nfcAdapter == null) {
                continuation.resumeWithException(Exception("NFC not available on this device"))
                return@suspendCancellableCoroutine
            }

            if (!nfcAdapter.isEnabled) {
                continuation.resumeWithException(Exception("NFC is disabled. Please enable NFC in settings"))
                return@suspendCancellableCoroutine
            }

            android.util.Log.d(tag, "Starting NFC scan for command: $cmd")

            // enable reader mode to detect ISO7816 tags (TapSigner uses NFC-A protocol)
            val readerCallback = NfcAdapter.ReaderCallback { tag ->
                android.util.Log.d(this.tag, "NFC tag detected")

                // execute NFC operation in coroutine scope
                CoroutineScope(Dispatchers.IO).launch {
                    try {
                        val result = executeNfcCommand(tag, cmd, successResult)
                        continuation.resume(result)
                    } catch (e: Exception) {
                        android.util.Log.e(this@TapSignerNfcHelper.tag, "NFC command failed", e)
                        continuation.resumeWithException(e)
                    } finally {
                        nfcAdapter.disableReaderMode(activity)
                    }
                }
            }

            // NFC_A: TapSigner uses NFC-A protocol
            // SKIP_NDEF_CHECK: we're sending raw APDU commands, not reading NDEF records
            nfcAdapter.enableReaderMode(
                activity,
                readerCallback,
                NfcAdapter.FLAG_READER_NFC_A or NfcAdapter.FLAG_READER_SKIP_NDEF_CHECK,
                null
            )

            // handle cancellation
            continuation.invokeOnCancellation {
                android.util.Log.d(tag, "NFC scan cancelled")
                nfcAdapter.disableReaderMode(activity)
            }
        }

    /**
     * Executes a TapSigner command using an NFC tag.
     *
     * Establishes IsoDep connection, creates transport and reader, executes the command,
     * and ensures proper cleanup of resources.
     *
     * @param T Expected result type
     * @param tag NFC tag detected by reader mode
     * @param cmd TapSigner command to execute
     * @param successResult Lambda to extract typed result from TapSignerResponse
     * @return Command result of type T
     * @throws Exception if tag is incompatible, connection fails, or command fails
     */
    private suspend fun <T> executeNfcCommand(
        tag: Tag,
        cmd: TapSignerCmd,
        successResult: (TapSignerResponse?) -> T?,
    ): T {
        val isoDep = IsoDep.get(tag)
            ?: throw Exception("Tag is not ISO7816 compatible")

        try {
            isoDep.connect()
            // increased timeout because TapSigner operations can be slow (default is 1000ms)
            isoDep.timeout = 2000

            android.util.Log.d(this.tag, "IsoDep connected, creating transport")

            // create transport and reader
            val transport = TapCardTransport(isoDep)
            val reader = tapSignerReaderNew(transport, cmd)

            android.util.Log.d(this.tag, "Running TapSigner command")
            val response = reader.run()
            lastResponse = response

            android.util.Log.d(this.tag, "Command completed, response: ${response::class.simpleName}")

            val result = successResult(response)
            if (result != null) {
                return result
            } else {
                throw Exception("Unexpected response type: ${response::class.simpleName}")
            }
        } finally {
            try {
                if (isoDep.isConnected) {
                    isoDep.close()
                }
            } catch (e: Exception) {
                android.util.Log.w(this.tag, "Error closing IsoDep", e)
            }
        }
    }

    /**
     * Executes multi-step TapSigner setup with retry logic.
     *
     * Setup involves multiple NFC operations (backup, derive, PIN change) that are
     * orchestrated by continueSetup(). This method handles retries for transient
     * failures, attempting up to 5 times before giving up.
     *
     * @param factoryPin Factory default PIN
     * @param newPin New PIN to set
     * @param chainCode Optional custom chain code
     * @return SetupCmdResponse.Complete with derived key info on success
     * @throws Exception if setup fails after maximum retry attempts
     */
    private suspend fun doSetupTapSigner(
        factoryPin: String,
        newPin: String,
        chainCode: ByteArray?,
    ): SetupCmdResponse {
        var errorCount = 0
        var lastError: Exception? = null

        // create initial setup command
        val setupCmd = try {
            SetupCmd.tryNew(factoryPin, newPin, chainCode)
        } catch (e: Exception) {
            android.util.Log.e(tag, "Failed to create setup command", e)
            throw e
        }

        // perform initial setup
        var response = try {
            performTapSignerCmd(TapSignerCmd.Setup(setupCmd)) { resp ->
                (resp as? TapSignerResponse.Setup)?.v1
            }
        } catch (e: Exception) {
            android.util.Log.e(tag, "Setup failed", e)
            throw e
        }

        // if already complete, return early
        if (response is SetupCmdResponse.Complete) {
            return response
        }

        // continue setup for multi-step flow (backup → derive → change PIN)
        while (true) {
            lastResponse = TapSignerResponse.Setup(response)

            val continueResult = try {
                continueSetup(response)
            } catch (e: Exception) {
                errorCount++
                lastError = e
                android.util.Log.e(tag, "Continue setup failed (attempt $errorCount)", e)

                if (errorCount > 5) {
                    android.util.Log.e(tag, "Max retries exceeded, last error: $lastError")
                    throw e
                }
                continue
            }

            when (continueResult) {
                is SetupCmdResponse.Complete -> {
                    return continueResult
                }
                else -> {
                    response = continueResult
                }
            }
        }
    }

    /**
     * Handles multi-step setup flow by determining and executing the next command.
     *
     * Setup progresses through states:
     * 1. ContinueFromInit → send backup command
     * 2. ContinueFromBackup → send derive command
     * 3. ContinueFromDerive → send change PIN command
     * 4. Complete → done
     *
     * Each step requires another NFC tap to execute the next command.
     *
     * @param response Current setup response state
     * @return Next SetupCmdResponse or Complete if finished
     * @throws Exception if NFC operation fails
     */
    suspend fun continueSetup(response: SetupCmdResponse): SetupCmdResponse {
        // extract the next command from the current state
        val cmd =
            when (response) {
                is SetupCmdResponse.ContinueFromInit -> response.v1.continueCmd
                is SetupCmdResponse.ContinueFromBackup -> response.v1.continueCmd
                is SetupCmdResponse.ContinueFromDerive -> response.v1.continueCmd
                is SetupCmdResponse.Complete -> null
            }

        if (cmd == null) return response

        android.util.Log.d(tag, "Continuing setup with next command")

        return performTapSignerCmd(TapSignerCmd.Setup(cmd)) { resp ->
            (resp as? TapSignerResponse.Setup)?.v1
        }
    }
}

/**
 * Android IsoDep transport implementation for TapSigner APDU communication.
 *
 * Implements TapcardTransportProtocol callback interface defined in Rust.
 * The Rust TapSignerReader calls back to this transport to send APDU commands
 * to the physical card via Android's IsoDep API.
 *
 * Unlike iOS, Android cannot update UI messages during an active NFC transaction,
 * so setMessage/appendMessage are no-ops that only log.
 *
 * @property isoDep Connected IsoDep tag for APDU transmission
 */
private class TapCardTransport(
    private val isoDep: IsoDep,
) : TapcardTransportProtocol {
    /**
     * Sets user-facing message during NFC operation.
     *
     * Android NFC doesn't support updating UI during reader mode callback,
     * so this is a no-op that only logs. iOS can update NFCTagReaderSession.alertMessage.
     */
    override fun setMessage(message: String) {
        android.util.Log.d("TapCardTransport", "Message: $message")
    }

    /**
     * Appends to user-facing message during NFC operation.
     *
     * Android NFC doesn't support updating UI during reader mode callback,
     * so this is a no-op that only logs.
     */
    override fun appendMessage(message: String) {
        android.util.Log.d("TapCardTransport", "Append: $message")
    }

    /**
     * Transmits APDU command to TapSigner and returns response.
     *
     * Uses IsoDep.transceive() which automatically appends status words (SW1, SW2)
     * to the response data, matching the format expected by Rust TapSignerReader.
     *
     * @param commandApdu APDU command bytes to send
     * @return APDU response including data and status words
     * @throws Exception if transmission fails or connection is lost
     */
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
