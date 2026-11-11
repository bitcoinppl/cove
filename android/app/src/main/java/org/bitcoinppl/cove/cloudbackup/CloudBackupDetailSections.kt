package org.bitcoinppl.cove.cloudbackup

import org.bitcoinppl.cove_core.CloudBackupInventoryIncompleteReason

internal fun cloudBackupOtherBackupsFailureMessage(
    reason: CloudBackupInventoryIncompleteReason,
): String =
    when (reason) {
        CloudBackupInventoryIncompleteReason.PROVIDER_SYNC_PENDING ->
            "Cove could not check for backup sets made with another backup key because Google Drive is still syncing."
        CloudBackupInventoryIncompleteReason.OFFLINE ->
            "Cove could not check for backup sets made with another backup key because this device is offline."
        CloudBackupInventoryIncompleteReason.AUTHORIZATION_REQUIRED ->
            "Cove cannot check for backup sets made with another backup key. Reconnect Google Drive, then check again."
        CloudBackupInventoryIncompleteReason.PROVIDER_UNAVAILABLE ->
            "Cove cannot check for backup sets made with another backup key because Google Drive is not available now."
        CloudBackupInventoryIncompleteReason.UNKNOWN ->
            "Cove could not check for backup sets made with another backup key."
    }
