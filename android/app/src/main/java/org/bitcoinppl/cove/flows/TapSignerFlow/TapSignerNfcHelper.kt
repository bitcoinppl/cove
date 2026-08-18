package org.bitcoinppl.cove.flows.TapSignerFlow

import org.bitcoinppl.cove.nfc.TapCardNfcManager
import org.bitcoinppl.cove_core.CkTapException
import org.bitcoinppl.cove_core.SetupCmd
import org.bitcoinppl.cove_core.SetupCmdResponse
import org.bitcoinppl.cove_core.TapSignerCvc
import org.bitcoinppl.cove_core.TapSignerCmd
import org.bitcoinppl.cove_core.TapSignerOperationContinuation
import org.bitcoinppl.cove_core.TapSignerResponse
import org.bitcoinppl.cove_core.types.Psbt
import org.bitcoinppl.cove_core.tapSignerResponseBackupResponse
import org.bitcoinppl.cove_core.tapSignerResponseChangeResponse
import org.bitcoinppl.cove_core.tapSignerResponseDeriveResponse
import org.bitcoinppl.cove_core.tapSignerResponseRetryResponse
import org.bitcoinppl.cove_core.tapSignerResponseSetupResponse
import org.bitcoinppl.cove_core.tapSignerResponseSignResponse

/** Coordinates typed Rust TAPSIGNER commands with the Android NFC manager. */
class TapSignerNfcHelper {
    private val responseOwner = TapSignerResponseOwner()

    /** Set up a card with CVCs and an optional exact 32-byte chain code. */
    suspend fun setupTapSigner(
        factoryCvcHex: String,
        newCvcHex: String,
        chainCode: ByteArray?,
        callbacks: TapCardNfcManager.OperationCallbacks,
    ): SetupCmdResponse {
        val factoryCvc = TapSignerCvc.tryFromHex(factoryCvcHex)
        return try {
            val newCvc = TapSignerCvc.tryFromHex(newCvcHex)
            try {
                val setup = SetupCmd.tryNew(factoryCvc, newCvc, chainCode)
                try {
                    responseOwner.perform(TapSignerCmd.Setup(setup), callbacks).setupResponse()
                } finally {
                    setup.destroy()
                }
            } finally {
                newCvc.destroy()
            }
        } finally {
            factoryCvc.destroy()
        }
    }

    /** Derive wallet data from the card. */
    suspend fun derive(
        cvcHex: String,
        callbacks: TapCardNfcManager.OperationCallbacks,
    ): org.bitcoinppl.cove_core.DeriveInfo =
        withTapSignerCvc(cvcHex) { cvc ->
            responseOwner.perform(TapSignerCmd.Derive(cvc), callbacks).deriveResponse()
        }

    /** Change the card CVC. */
    suspend fun changePin(
        currentCvcHex: String,
        newCvcHex: String,
        callbacks: TapCardNfcManager.OperationCallbacks,
    ) {
        val currentCvc = TapSignerCvc.tryFromHex(currentCvcHex)
        try {
            val newCvc = TapSignerCvc.tryFromHex(newCvcHex)
            try {
                responseOwner
                    .perform(TapSignerCmd.Change(currentCvc, newCvc), callbacks)
                    .changeResponse()
            } finally {
                newCvc.destroy()
            }
        } finally {
            currentCvc.destroy()
        }
    }

    /** Retrieve and return a card backup. */
    suspend fun backup(
        cvcHex: String,
        callbacks: TapCardNfcManager.OperationCallbacks,
    ): ByteArray =
        withTapSignerCvc(cvcHex) { cvc ->
            responseOwner.perform(TapSignerCmd.Backup(cvc), callbacks).backupResponse()
        }

    /** Sign a PSBT with the card. */
    suspend fun sign(
        psbt: Psbt,
        cvcHex: String,
        callbacks: TapCardNfcManager.OperationCallbacks,
    ): Psbt =
        withTapSignerCvc(cvcHex) { cvc ->
            responseOwner.perform(TapSignerCmd.Sign(psbt, cvc), callbacks).signResponse()
        }

    /** Continue an opaque setup continuation returned by Rust. */
    suspend fun continueSetup(
        response: SetupCmdResponse,
        callbacks: TapCardNfcManager.OperationCallbacks,
    ): SetupCmdResponse {
        val continuation =
            (response as? SetupCmdResponse.Retry)?.v1
                ?: return response
        return try {
            responseOwner
                .perform(TapSignerCmd.ContinueSetup(continuation), callbacks)
                .setupResponse()
        } finally {
            response.destroy()
        }
    }

    /** Continue an opaque derive continuation returned by Rust. */
    suspend fun continueDerive(
        callbacks: TapCardNfcManager.OperationCallbacks,
    ): org.bitcoinppl.cove_core.DeriveInfo {
        val continuation = responseOwner.operationContinuation()
        return responseOwner
            .perform(TapSignerCmd.ContinueOperation(continuation), callbacks)
            .deriveResponse()
    }

    /** Return the last response while the helper owns it. */
    fun lastResponse(): TapSignerResponse? = responseOwner.lastResponse()

    /** Return a cloned setup response for a route that may outlive the NFC helper. */
    fun lastSetupResponse(): SetupCmdResponse? =
        responseOwner.lastSetupResponse()

    /** Whether the last operation returned an opaque mutation continuation. */
    fun hasOperationContinuation(): Boolean = responseOwner.hasOperationContinuation()

    /** Release response and opaque continuation resources and cancel active NFC work. */
    fun close() {
        responseOwner.close()
    }
}

private class TapSignerResponseOwner {
    private val nfcManager = TapCardNfcManager.getInstance()
    private var lastResponse: TapSignerResponse? = null

    suspend fun perform(
        cmd: TapSignerCmd,
        callbacks: TapCardNfcManager.OperationCallbacks,
    ): TapSignerResponse {
        val previousResponse = lastResponse
        lastResponse = null

        val response = try {
            nfcManager.performTapSignerCmd(cmd, callbacks)
        } finally {
            previousResponse?.destroy()
        }

        lastResponse = response
        return response
    }

    fun operationContinuation(): TapSignerOperationContinuation =
        (lastResponse as? TapSignerResponse.Retry)?.v1
            ?: error("No TAPSIGNER operation continuation is available")

    fun lastResponse(): TapSignerResponse? = lastResponse

    fun lastSetupResponse(): SetupCmdResponse? =
        lastResponse?.let(::tapSignerResponseSetupResponse)

    fun hasOperationContinuation(): Boolean = lastResponse is TapSignerResponse.Retry

    fun close() {
        lastResponse?.destroy()
        lastResponse = null
        nfcManager.cancelActiveOperation()
    }
}

private suspend fun <T> withTapSignerCvc(
    hex: String,
    block: suspend (TapSignerCvc) -> T,
): T {
    val cvc = TapSignerCvc.tryFromHex(hex)
    return try {
        block(cvc)
    } finally {
        cvc.destroy()
    }
}

private fun TapSignerResponse.setupResponse(): SetupCmdResponse =
    tapSignerResponseSetupResponse(this)
        ?: when (this) {
            is TapSignerResponse.Retry -> throw TapSignerOperationRetryException(v1.message())
            else -> throw unexpectedResponse("setup")
        }

private fun TapSignerResponse.deriveResponse(): org.bitcoinppl.cove_core.DeriveInfo =
    tapSignerResponseDeriveResponse(this)
        ?: when (this) {
            is TapSignerResponse.Retry -> throw TapSignerOperationRetryException(v1.message())
            else -> throw unexpectedResponse("derive")
        }

private fun TapSignerResponse.changeResponse() {
    if (tapSignerResponseChangeResponse(this)) return

    when (this) {
        is TapSignerResponse.Retry -> throw TapSignerOperationRetryException(v1.message())
        else -> throw unexpectedResponse("change")
    }
}

private fun TapSignerResponse.backupResponse(): ByteArray =
    tapSignerResponseBackupResponse(this)
        ?: when (this) {
            is TapSignerResponse.Retry -> throw TapSignerOperationRetryException(v1.message())
            else -> throw unexpectedResponse("backup")
        }

private fun TapSignerResponse.signResponse(): Psbt =
    tapSignerResponseSignResponse(this)
        ?: when (this) {
            is TapSignerResponse.Retry -> throw TapSignerOperationRetryException(v1.message())
            else -> throw unexpectedResponse("sign")
        }

private fun unexpectedResponse(operation: String): IllegalStateException =
    IllegalStateException("Unexpected response for TAPSIGNER $operation")

/** Signals that Rust returned an opaque continuation requiring another card scan. */
internal class TapSignerOperationRetryException(
    message: String,
) : Exception(message)

internal enum class TapSignerFailureDisposition {
    AUTHENTICATION,
    OTHER,
    CANCELLATION,
}

internal fun classifyTapSignerFailure(error: Throwable): TapSignerFailureDisposition =
    when {
        error is kotlinx.coroutines.CancellationException -> TapSignerFailureDisposition.CANCELLATION
        isAuthError(error) -> TapSignerFailureDisposition.AUTHENTICATION
        else -> TapSignerFailureDisposition.OTHER
    }

internal fun isAuthError(error: Throwable): Boolean =
    error is org.bitcoinppl.cove_core.TapSignerReaderException.TapSignerException &&
        error.v1 is org.bitcoinppl.cove_core.TransportException.CkTap &&
        error.v1.v1 is CkTapException.BadAuth

internal fun isNoBackupError(error: Throwable): Boolean =
    error is org.bitcoinppl.cove_core.TapSignerReaderException.TapSignerException &&
        error.v1 is org.bitcoinppl.cove_core.TransportException.CkTap &&
        error.v1.v1 is CkTapException.BackupFirst
