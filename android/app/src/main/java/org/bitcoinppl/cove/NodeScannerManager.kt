package org.bitcoinppl.cove

import android.util.Log
import androidx.compose.runtime.Stable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.cancelChildren
import kotlinx.coroutines.launch
import kotlinx.coroutines.runBlocking
import org.bitcoinppl.cove_core.FoundNode
import org.bitcoinppl.cove_core.Node
import org.bitcoinppl.cove_core.NodeScannerReconcileMessage
import org.bitcoinppl.cove_core.NodeScannerReconciler
import org.bitcoinppl.cove_core.NodeScannerResult
import org.bitcoinppl.cove_core.NodeScannerState
import org.bitcoinppl.cove_core.RustNodeScannerManager
import org.bitcoinppl.cove_core.TrustRequiredEndpoint
import java.io.Closeable
import java.util.concurrent.atomic.AtomicBoolean

/** Owns one screen-local local-node discovery lifecycle */
// the suspend wrappers do suspend through the generated bindings; detekt cannot
// resolve them because the generated module fails its type analysis
@Suppress("TooManyFunctions", "RedundantSuspendModifier")
@Stable
class NodeScannerManager internal constructor(
    private val rust: RustNodeScannerManager,
) : NodeScannerReconciler,
    Closeable {
    private val mainScope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)
    private val isClosed = AtomicBoolean(false)
    private val rustGuard =
        RustHandleGuard(
            ownerName = "NodeScannerManager",
            handleName = "RustNodeScannerManager",
            isClosed = isClosed,
        ) {
            Log.w(TAG, it)
        }

    var state by mutableStateOf<NodeScannerState>(NodeScannerState.Idle)
        private set

    constructor() : this(RustNodeScannerManager())

    init {
        withRust {
            listenForUpdates(this@NodeScannerManager)
        }
        state = withRust { state() }
    }

    private fun <T> withRust(block: RustNodeScannerManager.() -> T): T =
        rustGuard.withHandle(rust, block)

    private suspend fun <T> withRustSuspend(block: suspend RustNodeScannerManager.() -> T): T =
        rustGuard.withHandleSuspend(rust, block)

    private suspend fun <T> withRustOrSuspend(
        defaultValue: T,
        block: suspend RustNodeScannerManager.() -> T,
    ): T = rustGuard.withHandleOrSuspend(rust, defaultValue, block)

    suspend fun start() {
        withRustOrSuspend(Unit) {
            startScan()
        }
    }

    suspend fun stop() {
        withRustOrSuspend(Unit) {
            stopScan()
        }
    }

    suspend fun createNode(found: FoundNode): Node =
        withRustSuspend {
            createNode(found)
        }

    suspend fun trustAndCreateNode(candidate: TrustRequiredEndpoint): Node =
        withRustSuspend {
            trustAndCreateNode(candidate)
        }

    private fun apply(message: NodeScannerReconcileMessage) {
        when (message) {
            is NodeScannerReconcileMessage.ScanStarted -> applyScanStarted(message)
            is NodeScannerReconcileMessage.ScanProgress -> applyScanProgress(message)
            is NodeScannerReconcileMessage.ResultUpserted -> applyResultUpserted(message)
            is NodeScannerReconcileMessage.ResultRemoved -> applyResultRemoved(message)
            is NodeScannerReconcileMessage.ScanStopped -> applyScanStopped(message)
            is NodeScannerReconcileMessage.ScanFinished -> applyScanFinished(message)
            is NodeScannerReconcileMessage.ScanFailed -> applyScanFailed(message)
        }
    }

    private fun applyScanStarted(message: NodeScannerReconcileMessage.ScanStarted) {
        val currentGeneration = currentGeneration
        if (currentGeneration != null && message.generation <= currentGeneration) return

        state =
            NodeScannerState.Scanning(
                generation = message.generation,
                network = message.network,
                progress = message.progress,
                results = emptyList(),
            )
    }

    private fun applyScanProgress(message: NodeScannerReconcileMessage.ScanProgress) {
        val current = scanningState(message.generation) ?: return
        state = current.copy(progress = message.progress)
    }

    private fun applyResultUpserted(message: NodeScannerReconcileMessage.ResultUpserted) {
        state =
            updatingResults(message.generation) { results ->
                val index = results.indexOfFirst { resultIp(it) == resultIp(message.result) }

                if (index >= 0) {
                    results[index] = message.result
                } else {
                    results.add(message.result)
                }
            } ?: return
    }

    private fun applyResultRemoved(message: NodeScannerReconcileMessage.ResultRemoved) {
        state =
            updatingResults(message.generation) { results ->
                results.removeAll { resultIp(it) == message.ip }
            } ?: return
    }

    private fun applyScanStopped(message: NodeScannerReconcileMessage.ScanStopped) {
        val current = scanningState(message.generation) ?: return

        state =
            NodeScannerState.Stopped(
                generation = current.generation,
                network = current.network,
                results = current.results,
            )
    }

    private fun applyScanFinished(message: NodeScannerReconcileMessage.ScanFinished) {
        val current = scanningState(message.generation) ?: return

        state =
            NodeScannerState.Finished(
                generation = current.generation,
                network = current.network,
                results = current.results,
            )
    }

    private fun applyScanFailed(message: NodeScannerReconcileMessage.ScanFailed) {
        val current = scanningState(message.generation) ?: return

        state =
            NodeScannerState.Failed(
                generation = current.generation,
                network = current.network,
                error = message.error,
                results = current.results,
            )
    }

    private fun scanningState(generation: ULong): NodeScannerState.Scanning? =
        (state as? NodeScannerState.Scanning)?.takeIf { it.generation == generation }

    private val currentGeneration: ULong?
        get() =
            when (val current = state) {
                NodeScannerState.Idle -> null
                is NodeScannerState.Scanning -> current.generation
                is NodeScannerState.Stopped -> current.generation
                is NodeScannerState.Finished -> current.generation
                is NodeScannerState.Failed -> current.generation
            }

    private fun updatingResults(
        generation: ULong,
        update: (MutableList<NodeScannerResult>) -> Unit,
    ): NodeScannerState? =
        when (val current = state) {
            NodeScannerState.Idle,
            is NodeScannerState.Failed,
            -> {
                null
            }

            is NodeScannerState.Scanning -> {
                if (current.generation != generation) return null

                current.copy(results = current.results.updated(update))
            }

            is NodeScannerState.Stopped -> {
                if (current.generation != generation) return null

                current.copy(results = current.results.updated(update))
            }

            is NodeScannerState.Finished -> {
                if (current.generation != generation) return null

                current.copy(results = current.results.updated(update))
            }
        }

    private fun List<NodeScannerResult>.updated(
        update: (MutableList<NodeScannerResult>) -> Unit,
    ): List<NodeScannerResult> = toMutableList().also(update)

    private fun resultIp(result: NodeScannerResult): String =
        when (result) {
            is NodeScannerResult.Verified -> result.v1.ip
            is NodeScannerResult.TrustRequired -> result.v1.ip
        }

    override fun reconcile(message: NodeScannerReconcileMessage) {
        if (isClosed.get()) return

        mainScope.launch {
            apply(message)
        }
    }

    override fun reconcileMany(messages: List<NodeScannerReconcileMessage>) {
        if (isClosed.get()) return

        mainScope.launch {
            messages.forEach(::apply)
        }
    }

    @Synchronized
    override fun close() {
        if (rustGuard.isClosed()) return

        mainScope.coroutineContext.cancelChildren()

        try {
            runBlocking {
                withRustOrSuspend(Unit) {
                    stopScan()
                }
            }
        } catch (error: IllegalStateException) {
            Log.w(TAG, "Failed to stop node scan while closing", error)
        } finally {
            rustGuard.closeOnce {
                rust.close()
            }
            mainScope.cancel()
        }
    }

    private companion object {
        const val TAG = "NodeScannerManager"
    }
}
