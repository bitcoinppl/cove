package org.bitcoinppl.cove

import androidx.compose.runtime.Stable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.launch
import org.bitcoinppl.cove_core.*
import org.bitcoinppl.cove_core.types.*
import java.io.Closeable
import java.util.concurrent.atomic.AtomicBoolean

/**
 * pending wallet manager - manages hot wallet creation flow
 * ported from iOS PendingWalletViewModel.swift
 */
@Stable
class PendingWalletManager(
    numberOfWords: NumberOfBip39Words,
) : PendingWalletManagerReconciler,
    Closeable {
    private val tag = "PendingWalletManager"

    private val mainScope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)
    private val isClosed = AtomicBoolean(false)

    val rust: RustPendingWalletManager = RustPendingWalletManager(numberOfWords)

    var numberOfWords by mutableStateOf(numberOfWords)
        private set

    var bip39Words by mutableStateOf<List<String>>(emptyList())
        private set

    init {
        logDebug("Initializing PendingWalletManager with $numberOfWords words")
        bip39Words = rust.bip39Words()
        rust.listenForUpdates(this)
    }

    private fun logDebug(message: String) {
        android.util.Log.d(tag, message)
    }

    override fun reconcile(message: PendingWalletManagerReconcileMessage) {
        logDebug("Reconcile: $message")
        mainScope.launch {
            when (message) {
                is PendingWalletManagerReconcileMessage.Words -> {
                    numberOfWords = message.v1
                    bip39Words = rust.bip39Words()
                }
            }
        }
    }

    fun dispatch(action: PendingWalletManagerAction) {
        logDebug("dispatch: $action")
        mainScope.launch(Dispatchers.IO) { rust.dispatch(action) }
    }

    override fun close() {
        if (!isClosed.compareAndSet(false, true)) return
        logDebug("Closing PendingWalletManager")
        bip39Words = emptyList()
        mainScope.cancel()
        rust.close()
    }
}
