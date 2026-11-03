package org.bitcoinppl.cove.test

import android.os.SystemClock
import android.util.Log
import androidx.test.platform.app.InstrumentationRegistry
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeout
import org.bitcoinppl.cove.AppManager
import org.bitcoinppl.cove.ImportWalletManager
import org.bitcoinppl.cove_core.Database
import org.bitcoinppl.cove_core.ImportWalletException
import org.bitcoinppl.cove_core.WalletMetadata
import org.bitcoinppl.cove_core.types.WalletId
import org.junit.Assert.assertEquals
import org.junit.Assert.fail
import java.util.concurrent.atomic.AtomicReference

private const val TAG = "TestWallets"
private const val POLL_INTERVAL_MILLIS = 50L
private const val DELETE_TIMEOUT_MILLIS = 30_000L

/**
 * A published BIP39 test vector, never used for funds, with the master fingerprint of its words
 *
 * No Kotlin or FFI call derives a fingerprint from words without importing them, so the fingerprint
 * is pinned here and checked against every import that creates the wallet
 */
class TestWalletWords(
    val words: List<String>,
    val masterFingerprint: String,
)

/**
 * Import a hot wallet from published test words, or return null when a wallet for them already
 * exists on this device
 *
 * A test that deletes its wallet afterwards cannot know who else relies on an existing one, and
 * importing words that match a watch-only or cold wallet upgrades that wallet to hot in place, so
 * the existing wallet is looked for without writing anything. Once that lookup finds none, an import
 * that still reports a duplicate fails the test instead of skipping it
 */
fun importTestWalletOrNull(wallet: TestWalletWords): WalletMetadata? {
    // import matches existing wallets by master fingerprint within the selected network and mode,
    // which is the list all() returns
    val existing = Database().wallets().all()
    if (existing.any { it.masterFingerprint?.asLowercase() == wallet.masterFingerprint }) return null

    val imported =
        try {
            ImportWalletManager().use { manager -> manager.importWallet(listOf(wallet.words)) }
        } catch (error: ImportWalletException.WalletAlreadyExists) {
            throw AssertionError("import reported an existing wallet that the read-only lookup did not find", error)
        }

    // a wrong pinned fingerprint would let the lookup above miss a wallet the import then upgrades
    assertEquals(
        "the pinned master fingerprint does not match the imported words",
        wallet.masterFingerprint,
        imported.masterFingerprint?.asLowercase(),
    )

    // the test deletes the wallet it gets back, so one that already existed must never reach it
    if (existing.any { it.id == imported.id }) {
        throw AssertionError("import returned a wallet that already existed")
    }

    return imported
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
