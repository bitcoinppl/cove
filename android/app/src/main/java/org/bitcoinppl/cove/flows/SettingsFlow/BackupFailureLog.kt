@file:Suppress("PackageNaming")

package org.bitcoinppl.cove.flows.SettingsFlow

import org.bitcoinppl.cove.Log
import org.bitcoinppl.cove_core.BackupException

private const val TAG = "Backup"

/**
 * Logs a backup command failure that did not come from the backup manager
 *
 * The Rust backup manager logs every `BackupException` it returns, so only other failures, such
 * as a launcher or binding error, would otherwise reach the screen without a log
 */
internal fun logUnexpectedBackupFailure(
    operation: String,
    error: Throwable,
) {
    if (error is BackupException) return

    Log.e(TAG, "Backup $operation failed outside the backup manager", error)
}
