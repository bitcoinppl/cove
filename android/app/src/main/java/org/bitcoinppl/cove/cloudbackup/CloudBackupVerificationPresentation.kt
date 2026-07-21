package org.bitcoinppl.cove.cloudbackup

import org.bitcoinppl.cove_core.CloudBackupSyncState

internal fun shouldShowCloudBackupSyncAction(
    hasNeedsSync: Boolean,
    syncState: CloudBackupSyncState?,
): Boolean = hasNeedsSync || syncState is CloudBackupSyncState.Failed
