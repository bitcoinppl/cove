package org.bitcoinppl.cove.nfc

import android.app.Activity
import android.nfc.NfcAdapter
import android.nfc.Tag
import android.nfc.tech.IsoDep
import android.os.Handler
import android.os.Looper
import java.io.IOException
import java.lang.ref.WeakReference
import java.util.concurrent.atomic.AtomicBoolean
import kotlin.coroutines.cancellation.CancellationException
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.Job
import kotlinx.coroutines.NonCancellable
import kotlinx.coroutines.ensureActive
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.coroutines.withContext
import kotlinx.coroutines.withTimeoutOrNull
import org.bitcoinppl.cove.Log
import org.bitcoinppl.cove_core.TapSignerCmd
import org.bitcoinppl.cove_core.TapSignerReader
import org.bitcoinppl.cove_core.TapSignerResponse
import org.bitcoinppl.cove_core.TapcardTransportProtocol
import org.bitcoinppl.cove_core.TransportException
import org.bitcoinppl.cove_core.createTapSignerReader

/** Owns one serialized stream of TAPSIGNER NFC operations. */
class TapCardNfcManager private constructor() {
    private val logTag = "TapCardNfcManager"
    private val mainHandler = Handler(Looper.getMainLooper())
    private val operationMutex = Mutex()
    private val operationGate = NfcOperationGate()

    private var activityRef: WeakReference<Activity>? = null
    private var nfcAdapter: NfcAdapter? = null
    private var activeSession: OperationSession? = null
    private var activeOperationJob: Job? = null
    private var readerOwner: OperationToken? = null
    private var pendingDisable: PendingDisable? = null

    /** Initialize the manager with the activity that owns reader mode. */
    fun initialize(activity: Activity) {
        activityRef = WeakReference(activity)
        nfcAdapter = NfcAdapter.getDefaultAdapter(activity)
        cancelPendingDisable()
        Log.d(logTag, "TapCardNfcManager initialized")
    }

    /** Callbacks for one NFC operation. They are never shared with another operation. */
    data class OperationCallbacks(
        val onMessageUpdate: ((String) -> Unit)? = null,
        val onTagDetected: (() -> Unit)? = null,
    )

    /** Perform one command and return its typed response. */
    suspend fun performTapSignerCmd(
        cmd: TapSignerCmd,
        callbacks: OperationCallbacks = OperationCallbacks(),
    ): TapSignerResponse =
        operationMutex.withLock {
            val activity = activityRef?.get() ?: throw TapCardNfcException("Activity no longer available")
            val adapter = nfcAdapter ?: throw TapCardNfcException("NFC is not available on this device")
            if (!adapter.isEnabled) {
                throw TapCardNfcException("NFC is disabled. Please enable it in Settings")
            }

            cancelPendingDisable()

            val token = operationGate.begin()
            val session = OperationSession(token, activity, adapter, callbacks)
            activeSession = session
            activeOperationJob = kotlinx.coroutines.currentCoroutineContext()[Job]
            readerOwner = token

            enableReaderMode(adapter, activity, session)

            var isoDep: IsoDep? = null
            var reader: TapSignerReader? = null

            try {
                val detectedTag =
                    withTimeoutOrNull(NFC_SCAN_TIMEOUT_MS) {
                        session.tagDetected.await()
                    } ?: throw TapCardNfcException("NFC scan timed out. Please try again")

                kotlinx.coroutines.currentCoroutineContext().ensureActive()
                Log.d(logTag, "Processing detected tag for operation ${token.id}")

                isoDep =
                    IsoDep.get(detectedTag)
                        ?: throw TapCardNfcException("Tag does not support IsoDep (ISO7816)")

                if (!isoDep.isConnected) {
                    isoDep.connect()
                }

                val timeout = timeoutFor(cmd)
                isoDep.timeout = timeout
                Log.d(logTag, "Connected to IsoDep tag (timeout=${timeout}ms, operation=${operationKind(cmd)})")

                if (timeout == TapCardTransport.ISODEP_LONG_TIMEOUT_MS) {
                    session.updateMessage("Keep your phone steady on the card — this may take a few seconds")
                }

                val transport = TapCardTransport(isoDep, session, timeout)
                reader = createTapSignerReader(transport, cmd)
                reader.run()
            } finally {
                withContext(NonCancellable) {
                    runCatching { reader?.close() }
                        .onFailure { Log.e(logTag, "Failed to close TapSigner reader", it) }
                    runCatching {
                        isoDep?.let {
                            if (it.isConnected) {
                                it.close()
                                Log.d(logTag, "IsoDep connection closed")
                            }
                        }
                    }.onFailure { Log.e(logTag, "Failed to close IsoDep connection", it) }

                    finishSession(session)
                    cmd.destroy()
                }
            }
        }

    /** Cancel the active operation without disabling NFC for a future operation. */
    fun cancelActiveOperation() {
        activeOperationJob?.cancel()
        activeSession?.tagDetected?.cancel(CancellationException("NFC operation cancelled"))

        val session = activeSession
        if (session != null) {
            session.cancel()
            mainHandler.post { scheduleReaderDisable(session) }
        }
    }

    /** Cancel active work and release reader-mode callbacks. */
    fun close() {
        cancelActiveOperation()
    }

    private fun enableReaderMode(
        adapter: NfcAdapter,
        activity: Activity,
        session: OperationSession,
    ) {
        runOnMain {
            if (!isCurrent(session)) return@runOnMain

            adapter.enableReaderMode(
                activity,
                { detectedTag -> session.onTagDetected(detectedTag) },
                NfcAdapter.FLAG_READER_NFC_A or
                    NfcAdapter.FLAG_READER_NFC_B or
                    NfcAdapter.FLAG_READER_SKIP_NDEF_CHECK or
                    NfcAdapter.FLAG_READER_NO_PLATFORM_SOUNDS,
                null,
            )
            Log.d(logTag, "NFC reader mode enabled for operation ${session.token.id}")
        }
    }

    private fun finishSession(session: OperationSession) {
        if (activeSession === session) {
            activeSession = null
            activeOperationJob = null
            operationGate.end(session.token)
        }

        mainHandler.post {
            scheduleReaderDisable(session)
        }
    }

    private fun scheduleReaderDisable(session: OperationSession) {
        val activity = session.activity
        if (readerOwner != session.token) return

        pendingDisable?.let { mainHandler.removeCallbacks(it.runnable) }

        val runnable =
            Runnable {
                if (readerOwner == session.token) {
                    session.adapter.disableReaderMode(activity)
                    readerOwner = null
                    pendingDisable = null
                    Log.d(logTag, "NFC reader mode disabled for operation ${session.token.id}")
                }
            }
        pendingDisable = PendingDisable(session.token, runnable)
        mainHandler.postDelayed(runnable, READER_MODE_DISABLE_DELAY_MS)
    }

    private fun cancelPendingDisable() {
        pendingDisable?.let { mainHandler.removeCallbacks(it.runnable) }
        pendingDisable = null
    }

    private fun isCurrent(session: OperationSession): Boolean =
        activeSession === session && operationGate.isCurrent(session.token) && !session.cancelled.get()

    private fun runOnMain(block: () -> Unit) {
        if (Looper.myLooper() == Looper.getMainLooper()) {
            block()
        } else {
            mainHandler.post(block)
        }
    }

    private inner class OperationSession(
        val token: OperationToken,
        val activity: Activity,
        val adapter: NfcAdapter,
        private val callbacks: OperationCallbacks,
    ) : NfcOperationSession {
        val tagDetected = CompletableDeferred<Tag>()
        val cancelled = AtomicBoolean(false)

        fun cancel() {
            cancelled.set(true)
        }

        fun onTagDetected(tag: Tag) {
            if (!isCurrent(this) || tagDetected.isCompleted) return

            postIfCurrent { callbacks.onTagDetected?.invoke() }
            tagDetected.complete(tag)
        }

        override fun updateMessage(message: String) {
            postIfCurrent { callbacks.onMessageUpdate?.invoke(message) }
        }

        private fun postIfCurrent(block: () -> Unit) {
            mainHandler.post {
                if (isCurrent(this)) block()
            }
        }
    }

    private data class PendingDisable(
        val token: OperationToken,
        val runnable: Runnable,
    )

    companion object {
        private const val NFC_SCAN_TIMEOUT_MS = 60_000L
        private const val READER_MODE_DISABLE_DELAY_MS = 5_000L

        @Volatile
        private var instance: TapCardNfcManager? = null

        fun getInstance(): TapCardNfcManager =
            instance ?: synchronized(this) {
                instance ?: TapCardNfcManager().also { instance = it }
            }
    }
}

internal data class OperationToken(val id: Long)

/** Small token gate used by the NFC manager and race-focused tests. */
internal class NfcOperationGate {
    private var nextId = 0L
    private var current: OperationToken? = null

    @Synchronized
    fun begin(): OperationToken {
        val token = OperationToken(++nextId)
        current = token
        return token
    }

    @Synchronized
    fun isCurrent(token: OperationToken): Boolean = current == token

    @Synchronized
    fun end(token: OperationToken) {
        if (current == token) current = null
    }
}

/** Android NFC transport for the Rust TAPSIGNER reader. */
private class TapCardTransport(
    private val isoDep: IsoDep,
    private val session: NfcOperationSession,
    private val timeoutMs: Int,
) : TapcardTransportProtocol {
    private val logTag = "TapCardTransport"
    private var currentMessage = ""

    override fun setMessage(message: String) {
        Log.d(logTag, "TapSigner progress message updated")
        currentMessage = message
        session.updateMessage(message)
    }

    override fun appendMessage(message: String) {
        Log.d(logTag, "TapSigner progress message appended")
        currentMessage += message
        session.updateMessage(currentMessage)
    }

    override suspend fun transmitApdu(commandApdu: ByteArray): ByteArray {
        Log.d(logTag, "Transmitting APDU: ${commandApdu.size} bytes")
        kotlinx.coroutines.currentCoroutineContext().ensureActive()

        return try {
            if (!isoDep.isConnected) {
                isoDep.connect()
                isoDep.timeout = timeoutMs
            }

            val response = isoDep.transceive(commandApdu)
            Log.d(logTag, "APDU response: ${response.size} bytes")
            response
        } catch (error: CancellationException) {
            throw error
        } catch (error: IOException) {
            Log.e(logTag, "TapSigner APDU transmission failed", error)
            throw TransportException.UnknownException(
                "Tag connection lost, please hold your phone still and try again",
            )
        }
    }

    companion object {
        const val ISODEP_TIMEOUT_MS = 5_000
        const val ISODEP_LONG_TIMEOUT_MS = 15_000
    }
}

private interface NfcOperationSession {
    fun updateMessage(message: String)
}

internal class TapCardNfcException(message: String) : Exception(message)

private fun timeoutFor(cmd: TapSignerCmd): Int =
    when (cmd) {
        is TapSignerCmd.Setup,
        is TapSignerCmd.ContinueSetup,
        is TapSignerCmd.ContinueOperation,
        is TapSignerCmd.Backup,
        is TapSignerCmd.Change,
        -> TapCardTransport.ISODEP_LONG_TIMEOUT_MS
        is TapSignerCmd.Derive,
        is TapSignerCmd.Sign,
        -> TapCardTransport.ISODEP_TIMEOUT_MS
    }

private fun operationKind(cmd: TapSignerCmd): String =
    when (cmd) {
        is TapSignerCmd.Setup -> "setup"
        is TapSignerCmd.ContinueSetup -> "continue_setup"
        is TapSignerCmd.ContinueOperation -> "continue_operation"
        is TapSignerCmd.Backup -> "backup"
        is TapSignerCmd.Derive -> "derive"
        is TapSignerCmd.Change -> "change"
        is TapSignerCmd.Sign -> "sign"
    }
