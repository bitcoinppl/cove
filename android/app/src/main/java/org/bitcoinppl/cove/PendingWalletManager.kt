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
    private val rustGuard =
        RustHandleGuard(
            ownerName = "PendingWalletManager",
            handleName = "RustPendingWalletManager",
            isClosed = isClosed,
        ) {
            android.util.Log.w(tag, it)
        }

    private val rust: RustPendingWalletManager = RustPendingWalletManager(numberOfWords)

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

    private fun <T> withRust(
        block: RustPendingWalletManager.() -> T,
    ): T = rustGuard.withHandle(rust, block)

    private fun <T> withRustOr(
        defaultValue: T,
        block: RustPendingWalletManager.() -> T,
    ): T = rustGuard.withHandleOr(rust, defaultValue, block)

    override fun reconcile(message: PendingWalletManagerReconcileMessage) {
        logDebug("Reconcile: $message")
        mainScope.launch {
            when (message) {
                is PendingWalletManagerReconcileMessage.Words -> {
                    numberOfWords = message.v1
                    bip39Words = withRustOr(emptyList()) {
                        bip39Words()
                    }
                }
            }
        }
    }

    fun dispatch(action: PendingWalletManagerAction) {
        logDebug("dispatch: $action")
        mainScope.launch(Dispatchers.IO) {
            withRustOr(Unit) {
                dispatch(action)
            }
        }
    }

    fun bip39WordsGrouped(): List<List<GroupedWord>> =
        withRustOr(emptyList()) {
            bip39WordsGrouped()
        }

    fun saveWallet(): PendingWalletSaveResult =
        withRust {
            saveWallet()
        }

    override fun close() {
        rustGuard.closeOnce {
            logDebug("Closing PendingWalletManager")
            bip39Words = emptyList()
            mainScope.cancel()
            rust.close()
        }
    }
}
