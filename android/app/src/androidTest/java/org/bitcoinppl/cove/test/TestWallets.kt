package org.bitcoinppl.cove.test

import android.os.SystemClock
import android.util.Log
import androidx.test.platform.app.InstrumentationRegistry
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeout
import org.bitcoinppl.cove.AppManager
import org.bitcoinppl.cove.ImportWalletManager
import org.bitcoinppl.cove_core.ImportWalletException
import org.bitcoinppl.cove_core.WalletMetadata
import org.bitcoinppl.cove_core.types.WalletId
import org.junit.Assert.fail
import java.util.concurrent.atomic.AtomicReference

private const val TAG = "TestWallets"
private const val POLL_INTERVAL_MILLIS = 50L
private const val DELETE_TIMEOUT_MILLIS = 30_000L

/**
 * Import a hot wallet from published test words
 *
 * Returns null when the wallet already exists, since a test that deletes its wallet afterwards
 * cannot know who else relies on an existing one
 */
fun importTestWalletOrNull(words: List<String>): WalletMetadata? =
    ImportWalletManager().use { manager ->
        try {
            manager.importWallet(listOf(words))
        } catch (_: ImportWalletException.WalletAlreadyExists) {
            null
        }
    }

/** Delete a wallet a test imported, logging instead of failing so cleanup never hides the result */
fun deleteTestWallet(walletId: WalletId) {
    runCatching {
        runBlocking {
            withTimeout(DELETE_TIMEOUT_MILLIS) {
                val app = AppManager.getInstance()
                val manager = app.getWalletManagerLoaded(walletId)
                app.deleteWalletInOwnerScope(manager).await().getOrThrow()
            }
        }
    }.onFailure { Log.w(TAG, "failed to delete the test wallet", it) }
}

/**
 * Poll [read] on the main thread until it returns a value
 *
 * Manager state lives in Compose snapshot state that reconcilers update on the main thread, so
 * reading it there sees each update in order without needing a composition
 */
fun <T : Any> awaitOnMain(
    what: String,
    timeoutMillis: Long = 15_000,
    read: () -> T?,
): T {
    val instrumentation = InstrumentationRegistry.getInstrumentation()
    val deadline = SystemClock.uptimeMillis() + timeoutMillis
    val value = AtomicReference<T?>()

    while (true) {
        instrumentation.runOnMainSync { value.set(read()) }
        value.get()?.let { return it }
        if (SystemClock.uptimeMillis() > deadline) fail("timed out after ${timeoutMillis}ms waiting for $what")

        Thread.sleep(POLL_INTERVAL_MILLIS)
    }
}
