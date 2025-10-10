package org.bitcoinppl.cove

import android.util.Log
import androidx.compose.runtime.Stable

@Stable
class ImportWalletManager : ImportWalletManagerReconciler {
    private val tag = "ImportWalletManager"
    val rust: RustImportWalletManager

    init {
        Log.d(tag, "Initializing ImportWalletManager")
        rust = RustImportWalletManager()
        rust.listenForUpdates(this)
    }

    override fun reconcile(message: ImportWalletManagerReconcileMessage) {
        Log.d(tag, "Reconcile: $message")

        when (message) {
            ImportWalletManagerReconcileMessage.NO_OP -> {
                // no-op
            }
        }
    }

    fun dispatch(action: ImportWalletManagerAction) {
        Log.d(tag, "Dispatch: $action")
        rust.dispatch(action)
    }

    /**
     * Import wallet from entered words
     * @param enteredWords List of lists of words (for paginated input)
     * @return WalletMetadata for the imported wallet
     * @throws ImportWalletException if import fails
     */
    fun importWallet(enteredWords: List<List<String>>): WalletMetadata {
        Log.d(tag, "Importing wallet with ${enteredWords.flatten().size} words")
        return rust.importWallet(enteredWords)
    }
}
