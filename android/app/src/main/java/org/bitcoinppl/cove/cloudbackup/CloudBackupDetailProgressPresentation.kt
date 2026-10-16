package org.bitcoinppl.cove.cloudbackup

import org.bitcoinppl.cove_core.CloudBackupDetail
import org.bitcoinppl.cove_core.CloudOnlyState

internal enum class CloudBackupDetailProgressPresentation {
    NONE,
    INVENTORY_INLINE,
    VERIFICATION_CARD,
    VERIFICATION_INLINE,
}

internal fun cloudBackupDetailProgressPresentation(
    isVerificationRunning: Boolean,
    isInventoryChecking: Boolean,
    hasRetainedDetail: Boolean,
    hasVisibleWalletRows: Boolean,
): CloudBackupDetailProgressPresentation =
    when {
        isVerificationRunning && hasVisibleWalletRows ->
            CloudBackupDetailProgressPresentation.VERIFICATION_INLINE
        isVerificationRunning -> CloudBackupDetailProgressPresentation.VERIFICATION_CARD
        isInventoryChecking && hasRetainedDetail ->
            CloudBackupDetailProgressPresentation.INVENTORY_INLINE
        else -> CloudBackupDetailProgressPresentation.NONE
    }

internal fun cloudBackupHasVisibleWalletRows(
    detail: CloudBackupDetail?,
    cloudOnly: CloudOnlyState,
): Boolean =
    detail?.let {
        it.upToDate.isNotEmpty() ||
            it.needsSync.isNotEmpty() ||
            // an incomplete list shows Check Again even with no rows, so verification must stay inline
            (cloudOnly as? CloudOnlyState.Loaded)?.let { loaded ->
                loaded.incomplete || loaded.wallets.isNotEmpty()
            } == true
    } == true
