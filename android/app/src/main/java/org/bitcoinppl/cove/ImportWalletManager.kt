package org.bitcoinppl.cove

import androidx.compose.runtime.Stable
import org.bitcoinppl.cove_core.*
import org.bitcoinppl.cove_core.types.*
import java.io.Closeable
import java.util.concurrent.atomic.AtomicBoolean

@Stable
class ImportWalletManager : Closeable {
    private val tag = "ImportWalletManager"
    private val isClosed = AtomicBoolean(false)
    private val rustGuard =
        RustHandleGuard(
            ownerName = "ImportWalletManager",
            handleName = "RustImportWalletManager",
            isClosed = isClosed,
        ) {
            Log.w(tag, it)
        }

    private val rust: RustImportWalletManager

    init {
        Log.d(tag, "Initializing ImportWalletManager")
        rust = RustImportWalletManager()
    }

    private fun <T> withRust(
        block: RustImportWalletManager.() -> T,
    ): T = rustGuard.withHandle(rust, block)

    /**
     * Import wallet from entered words
     * @param enteredWords List of lists of words (for paginated input)
     * @return WalletMetadata for the imported wallet
     * @throws ImportWalletException if import fails
     */
    fun importWallet(enteredWords: List<List<String>>): WalletMetadata {
        Log.d(tag, "Importing wallet with ${enteredWords.flatten().size} words")

        return withRust {
            importWallet(enteredWords)
        }
    }

    override fun close() {
        rustGuard.closeOnce {
            Log.d(tag, "Closing ImportWalletManager")
            rust.close()
        }
    }
}
