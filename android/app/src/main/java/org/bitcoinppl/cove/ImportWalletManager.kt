package org.bitcoinppl.cove

import androidx.compose.runtime.Stable
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.launch
import org.bitcoinppl.cove_core.*
import org.bitcoinppl.cove_core.types.*
import java.io.Closeable
import java.util.concurrent.atomic.AtomicBoolean

@Stable
class ImportWalletManager :
    ImportWalletManagerReconciler,
    Closeable {
    private val tag = "ImportWalletManager"
    private val isClosed = AtomicBoolean(false)
    private val mainScope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)

    val rust: RustImportWalletManager

    init {
        Log.d(tag, "Initializing ImportWalletManager")
        rust = RustImportWalletManager()
        rust.listenForUpdates(this)
    }

    override fun reconcile(message: ImportWalletManagerReconcileMessage) {
        Log.d(tag, "Reconcile: $message")
        mainScope.launch {
            when (message) {
                ImportWalletManagerReconcileMessage.NO_OP -> {
                    // no-op
                }
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

    override fun close() {
        if (!isClosed.compareAndSet(false, true)) return
        Log.d(tag, "Closing ImportWalletManager")
        mainScope.cancel()
        rust.close()
    }
}
