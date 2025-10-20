package org.bitcoinppl.cove

import androidx.compose.runtime.Stable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.GlobalScope
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

/**
 * pending wallet manager - manages hot wallet creation flow
 * ported from iOS PendingWalletViewModel.swift
 */
@Stable
class PendingWalletManager(
    numberOfWords: NumberOfBip39Words,
) : PendingWalletManagerReconciler {
    private val tag = "PendingWalletManager"

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
        GlobalScope.launch(Dispatchers.IO) {
            logDebug("Reconcile: $message")
            withContext(Dispatchers.Main) {
                when (message) {
                    is PendingWalletManagerReconcileMessage.Words -> {
                        numberOfWords = message.numberOfBip39Words
                        bip39Words = rust.bip39Words()
                    }
                }
            }
        }
    }

    fun dispatch(action: PendingWalletManagerAction) {
        GlobalScope.launch(Dispatchers.IO) {
            logDebug("dispatch: $action")
            rust.dispatch(action)
        }
    }
}
