package org.bitcoinppl.cove.testconfig

import android.content.Context
import org.bitcoinppl.cove.KeychainAccessor
import org.bitcoinppl.cove_core.device.KeychainAccess

/**
 * The app's real keychain, with a hold a test can put on the delete that ends a Cloud Backup
 * enable's local promotion
 *
 * Every call goes to [KeychainAccessor], so keys survive a process restart as they do in the app
 */
object HoldableKeychainAccess : KeychainAccess {
    // rust's CSPP_PENDING_ENABLE_JOURNAL_KEY
    private const val PENDING_ENABLE_JOURNAL_KEY = "cspp::v1::pending_enable_journal"

    @Volatile
    private var keychain: KeychainAccess? = null

    /** Held before the keychain deletes the pending enable journal, after the promotion committed */
    val pendingEnableJournalDelete = TestHold()

    /** Whether an unfinished enable still owns its staged key through the pending enable journal */
    fun hasPendingEnableJournal(): Boolean = keychain().get(PENDING_ENABLE_JOURNAL_KEY) != null

    fun attach(context: Context): KeychainAccess {
        keychain = KeychainAccessor(context)
        return this
    }

    override fun save(
        key: String,
        value: String,
    ) = keychain().save(key, value)

    override fun get(key: String): String? = keychain().get(key)

    override fun delete(key: String): Boolean {
        // only a journal that exists can end a promotion
        if (key == PENDING_ENABLE_JOURNAL_KEY && keychain().get(key) != null) {
            pendingEnableJournalDelete.enterIfHeld()
        }

        return keychain().delete(key)
    }

    override fun deleteAllWalletItems() = keychain().deleteAllWalletItems()

    private fun keychain(): KeychainAccess = requireNotNull(keychain) { "the keychain is used before attach" }
}
