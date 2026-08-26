package org.bitcoinppl.cove.flows.TapSignerFlow

import kotlinx.coroutines.currentCoroutineContext
import kotlinx.coroutines.ensureActive
import org.bitcoinppl.cove.nfc.TapCardNfcException
import org.bitcoinppl.cove.nfc.TapCardNfcManager
import org.bitcoinppl.cove_core.CkTapException
import org.bitcoinppl.cove_core.SetupCmd
import org.bitcoinppl.cove_core.SetupCmdResponse
import org.bitcoinppl.cove_core.TapSignerCvc
import org.bitcoinppl.cove_core.TapSignerCmd
import org.bitcoinppl.cove_core.TapSignerCommandResolution
import org.bitcoinppl.cove_core.TapSignerOperationContinuation
import org.bitcoinppl.cove_core.TapSignerReaderException
import org.bitcoinppl.cove_core.TapSignerResponse
import org.bitcoinppl.cove_core.TransportException
import org.bitcoinppl.cove_core.types.Psbt
import org.bitcoinppl.cove_core.resolveTapSignerCommand
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
        factoryCvc: String,
        newCvc: String,
        chainCode: ByteArray?,
        callbacks: TapCardNfcManager.OperationCallbacks,
    ): SetupCmdResponse {
        currentCoroutineContext().ensureActive()

        val factoryCvcObject = TapSignerCvc.tryNew(factoryCvc)
        return try {
            val newCvcObject = TapSignerCvc.tryNew(newCvc)
            try {
                val setup = SetupCmd.tryNew(factoryCvcObject, newCvcObject, chainCode)
                try {
                    responseOwner.perform(
                        TapSignerCmd.Setup(setup.cloneForTapSignerCommand()),
                        callbacks,
                    ) { response ->
                        response.setupResponse()
                    }
                } finally {
                    setup.destroy()
                }
            } finally {
                newCvcObject.destroy()
            }
        } finally {
            factoryCvcObject.destroy()
        }
    }

    /** Derive wallet data from the card. */
    suspend fun derive(
        cvc: String,
        callbacks: TapCardNfcManager.OperationCallbacks,
    ): org.bitcoinppl.cove_core.DeriveInfo {
        currentCoroutineContext().ensureActive()

        return withTapSignerCvc(cvc) { cvcObject ->
            responseOwner.performOperation(
                "derive",
                callbacks,
                initialCommand = { TapSignerCmd.Derive(cvcObject.cloneForTapSignerCommand()) },
            ) { response ->
                response.deriveResponse()
            }
        }
    }

    /** Change the card CVC. */
    suspend fun changePin(
        currentCvc: String,
        newCvc: String,
        callbacks: TapCardNfcManager.OperationCallbacks,
    ) {
        currentCoroutineContext().ensureActive()

        val currentCvcObject = TapSignerCvc.tryNew(currentCvc)
        try {
            val newCvcObject = TapSignerCvc.tryNew(newCvc)
            try {
                responseOwner.performOperation(
                    "change",
                    callbacks,
                    initialCommand = {
                        TapSignerCmd.Change(
                            currentCvcObject.cloneForTapSignerCommand(),
                            newCvcObject.cloneForTapSignerCommand(),
                        )
                    },
                ) { response ->
                    response.changeResponse()
                }
            } finally {
                newCvcObject.destroy()
            }
        } finally {
            currentCvcObject.destroy()
        }
    }

    /** Retrieve and return a card backup. */
    suspend fun backup(
        cvc: String,
        callbacks: TapCardNfcManager.OperationCallbacks,
    ): ByteArray {
        currentCoroutineContext().ensureActive()

        return withTapSignerCvc(cvc) { cvcObject ->
            responseOwner.performOperation(
                "backup",
                callbacks,
                initialCommand = { TapSignerCmd.Backup(cvcObject.cloneForTapSignerCommand()) },
            ) { response ->
                response.backupResponse()
            }
        }
    }

    /** Sign a PSBT with the card. */
    suspend fun sign(
        psbt: Psbt,
        cvc: String,
        callbacks: TapCardNfcManager.OperationCallbacks,
    ): Psbt {
        currentCoroutineContext().ensureActive()

        return withTapSignerCvc(cvc) { cvcObject ->
            responseOwner.performOperation(
                "sign",
                callbacks,
                // clone because generated commands own and destroy their nested handles
                initialCommand = {
                    TapSignerCmd.Sign(
                        psbt.cloneForTapSignerCommand(),
                        cvcObject.cloneForTapSignerCommand(),
                    )
                },
            ) { response ->
                response.signResponse()
            }
        }
    }

    /** Continue an opaque setup continuation returned by Rust. */
    suspend fun continueSetup(
        response: SetupCmdResponse,
        callbacks: TapCardNfcManager.OperationCallbacks,
    ): SetupCmdResponse {
        currentCoroutineContext().ensureActive()

        val continuation =
            (response as? SetupCmdResponse.Retry)?.v1
                ?: return response
        return responseOwner.performSetupContinuation(
            response = response,
            command = TapSignerCmd.ContinueSetup(continuation.cloneForTapSignerCommand()),
            callbacks = callbacks,
        ) { result ->
            result.setupResponse()
        }
    }

    /** Continue an opaque derive continuation returned by Rust. */
    suspend fun continueDerive(
        callbacks: TapCardNfcManager.OperationCallbacks,
    ): org.bitcoinppl.cove_core.DeriveInfo {
        currentCoroutineContext().ensureActive()

        return responseOwner.continueOperation("derive", callbacks) { response ->
            response.deriveResponse()
        }
    }

    /** Return a cloned setup response for a route that may outlive the NFC helper. */
    fun lastSetupResponse(): SetupCmdResponse? =
        responseOwner.lastSetupResponse()

    /** Release response and opaque continuation resources and cancel active NFC work. */
    fun close() {
        responseOwner.close()
    }
}

private class TapSignerResponseOwner {
    private val nfcManager = TapCardNfcManager.getInstance()
    private val operationToken = TapCardNfcManager.OperationToken()
    private val responseStore = TapSignerResponseStore()
    private var pendingSetupResponse: SetupCmdResponse? = null

    suspend fun <T> perform(
        cmd: TapSignerCmd,
        callbacks: TapCardNfcManager.OperationCallbacks,
        responseHandler: (TapSignerResponse) -> T,
    ): T {
        releasePendingSetupResponse()
        val generation = responseStore.prepareFresh()

        return performFresh(cmd, callbacks, generation, responseHandler)
    }

    suspend fun <T> performSetupContinuation(
        response: SetupCmdResponse,
        command: TapSignerCmd,
        callbacks: TapCardNfcManager.OperationCallbacks,
        responseHandler: (TapSignerResponse) -> T,
    ): T {
        synchronized(this) {
            if (pendingSetupResponse !== response) {
                pendingSetupResponse?.destroy()
                pendingSetupResponse = response
            }
        }

        val generation = responseStore.currentGeneration()
        return performFresh(command, callbacks, generation, responseHandler)
            .also { releasePendingSetupResponse() }
    }

    suspend fun <T> performOperation(
        label: String,
        callbacks: TapCardNfcManager.OperationCallbacks,
        initialCommand: () -> TapSignerCmd,
        responseHandler: (TapSignerResponse) -> T,
    ): T {
        releasePendingSetupResponse()
        val cmd = initialCommand()

        // Rust owns the routing: a pending continuation either resumes this
        // exact command or blocks a conflicting one, and is never silently dropped
        var routed = false
        val preparation =
            try {
                responseStore.prepareOperation(cmd, label).also { routed = true }
            } finally {
                if (!routed) cmd.destroy()
            }

        return when (preparation) {
            is PreparedTapSignerOperation.Fresh -> {
                performFresh(cmd, callbacks, preparation.generation, responseHandler)
            }
            is PreparedTapSignerOperation.Continuation -> {
                cmd.destroy()
                performContinuation(preparation.value, callbacks, responseHandler)
            }
        }
    }

    suspend fun <T> continueOperation(
        label: String,
        callbacks: TapCardNfcManager.OperationCallbacks,
        responseHandler: (TapSignerResponse) -> T,
    ): T {
        currentCoroutineContext().ensureActive()

        val preparation = responseStore.prepareContinuation(label)

        return performContinuation(preparation, callbacks, responseHandler)
    }

    private suspend fun <T> performFresh(
        cmd: TapSignerCmd,
        callbacks: TapCardNfcManager.OperationCallbacks,
        generation: Long,
        responseHandler: (TapSignerResponse) -> T,
    ): T {
        return try {
            val response = nfcManager.performTapSignerCmd(operationToken, cmd, callbacks)
            responseStore.acceptResponse(response, generation, responseHandler)
        } finally {
            cmd.destroy()
        }
    }

    private suspend fun <T> performContinuation(
        prepared: PreparedContinuation,
        callbacks: TapCardNfcManager.OperationCallbacks,
        responseHandler: (TapSignerResponse) -> T,
    ): T {
        return try {
            val response = nfcManager.performTapSignerCmd(operationToken, prepared.command, callbacks)
            responseStore.acceptResponse(
                response,
                prepared.generation,
                responseHandler,
                expectedPrevious = prepared.retry,
            )
        } catch (error: kotlinx.coroutines.CancellationException) {
            rethrowContinuationFailure(prepared, error)
        } catch (error: TapSignerReaderException) {
            rethrowContinuationFailure(prepared, error)
        } catch (error: TapCardNfcException) {
            // the reader did not start, so keep the continuation for the next scan
            throw error
        } catch (error: TransportException) {
            rethrowContinuationFailure(prepared, error)
        } finally {
            prepared.command.destroy()
        }
    }

    private fun rethrowContinuationFailure(
        prepared: PreparedContinuation,
        error: Exception,
    ): Nothing {
        responseStore.handleContinuationFailure(prepared)
        throw error
    }

    fun lastSetupResponse(): SetupCmdResponse? = responseStore.lastSetupResponse()

    fun close() {
        releasePendingSetupResponse()
        responseStore.close()
        nfcManager.cancelOperation(operationToken)
    }

    private fun releasePendingSetupResponse() {
        synchronized(this) {
            pendingSetupResponse?.destroy()
            pendingSetupResponse = null
        }
    }
}

private const val NFC_HELPER_CLOSED_MESSAGE = "TAPSIGNER NFC helper is closed"

private class TapSignerResponseStore {
    // keep close from destroying generated handles while state or responses are being inspected
    private val lifecycleLock = Any()
    private var lastResponse: TapSignerResponse? = null
    private var closed = false
    private var nextOperationGeneration = 0L

    fun prepareFresh(): Long = synchronized(lifecycleLock) {
        ensureOpenLocked()
        prepareFreshLocked()
    }

    fun currentGeneration(): Long = synchronized(lifecycleLock) {
        ensureOpenLocked()
        nextOperationGeneration
    }

    fun prepareOperation(
        cmd: TapSignerCmd,
        label: String,
    ): PreparedTapSignerOperation = synchronized(lifecycleLock) {
        ensureOpenLocked()

        val retry = lastResponse as? TapSignerResponse.Retry
            ?: return PreparedTapSignerOperation.Fresh(prepareFreshLocked())

        when (resolveTapSignerCommand(cmd, retry.v1)) {
            TapSignerCommandResolution.FRESH -> {
                PreparedTapSignerOperation.Fresh(prepareFreshLocked())
            }
            TapSignerCommandResolution.RESUME -> {
                PreparedTapSignerOperation.Continuation(prepareContinuationLocked(retry))
            }
            TapSignerCommandResolution.PENDING_OPERATION_CONFLICT -> {
                error("Complete the pending TAPSIGNER operation before starting $label")
            }
        }
    }

    fun prepareContinuation(label: String): PreparedContinuation =
        synchronized(lifecycleLock) {
            ensureOpenLocked()

            val retry = lastResponse as? TapSignerResponse.Retry
                ?: error("No TAPSIGNER $label continuation is available")

            prepareContinuationLocked(retry)
        }

    private fun prepareFreshLocked(): Long {
        val previousResponse = lastResponse
        lastResponse = null
        previousResponse?.destroy()
        return ++nextOperationGeneration
    }

    private fun prepareContinuationLocked(
        retry: TapSignerResponse.Retry,
    ): PreparedContinuation {
        val command = TapSignerCmd.ContinueOperation(retry.continuationClone())
        return PreparedContinuation(++nextOperationGeneration, retry, command)
    }

    fun <T> acceptResponse(
        response: TapSignerResponse,
        generation: Long,
        responseHandler: (TapSignerResponse) -> T,
        expectedPrevious: TapSignerResponse.Retry? = null,
    ): T = synchronized(lifecycleLock) {
        if (closed) {
            response.destroy()
            lastResponse = null
            throw kotlinx.coroutines.CancellationException(NFC_HELPER_CLOSED_MESSAGE)
        }

        if (generation != nextOperationGeneration ||
            (expectedPrevious != null && lastResponse !== expectedPrevious)
        ) {
            response.destroy()
            throw kotlinx.coroutines.CancellationException("TAPSIGNER operation was superseded")
        }

        val previousResponse = lastResponse
        lastResponse = response
        previousResponse?.destroy()

        responseHandler(response)
    }

    fun handleContinuationFailure(prepared: PreparedContinuation) {
        synchronized(lifecycleLock) {
            if (closed) {
                lastResponse = null
                return
            }

            if (prepared.generation != nextOperationGeneration ||
                lastResponse !== prepared.retry
            ) {
                return
            }

            if (!prepared.retry.v1.canRetry()) {
                lastResponse = null
                prepared.retry.destroy()
            }
        }
    }

    private fun ensureOpenLocked() {
        if (closed) throw kotlinx.coroutines.CancellationException(NFC_HELPER_CLOSED_MESSAGE)
    }

    fun lastSetupResponse(): SetupCmdResponse? =
        synchronized(lifecycleLock) {
            lastResponse?.let(::tapSignerResponseSetupResponse)
        }

    fun close() {
        synchronized(lifecycleLock) {
            val response = lastResponse
            lastResponse = null
            closed = true
            ++nextOperationGeneration
            response?.destroy()
        }
    }
}

private sealed interface PreparedTapSignerOperation {
    data class Fresh(val generation: Long) : PreparedTapSignerOperation

    data class Continuation(val value: PreparedContinuation) : PreparedTapSignerOperation
}

private data class PreparedContinuation(
    val generation: Long,
    val retry: TapSignerResponse.Retry,
    val command: TapSignerCmd,
)

private fun TapSignerResponse.Retry.continuationClone(): TapSignerOperationContinuation =
    tapSignerResponseRetryResponse(this)
        ?: error("TAPSIGNER retry response did not contain a continuation")

private suspend fun <T> withTapSignerCvc(
    value: String,
    block: suspend (TapSignerCvc) -> T,
): T {
    val cvc = TapSignerCvc.tryNew(value)
    return try {
        block(cvc)
    } finally {
        cvc.destroy()
    }
}

private fun TapSignerResponse.setupResponse(): SetupCmdResponse =
    tapSignerResponseSetupResponse(this) ?: throw unexpectedResponse("setup")

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
    tapSignerResponseSignResponse(this) ?: throw unexpectedResponse("sign")

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
