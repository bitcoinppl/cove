package org.bitcoinppl.cove.cloudbackup

import org.bitcoinppl.cove_core.CloudBackupInventoryIncompleteReason
import org.bitcoinppl.cove_core.CloudBackupOtherBackupsState
import org.bitcoinppl.cove_core.CloudBackupOtherBackupsSummary
import org.bitcoinppl.cove_core.CloudBackupRestoreAllState
import org.bitcoinppl.cove_core.CloudBackupWalletItem
import org.bitcoinppl.cove_core.CloudBackupWalletStatus
import org.bitcoinppl.cove_core.CloudOnlyState
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class CloudBackupSupplementalInventoryTest {
    @Test
    fun cloudOnlySectionRequiresNonemptyLoadedInventory() {
        assertNull(cloudBackupVisibleCloudOnlyRows(CloudOnlyState.NotFetched))
        assertNull(cloudBackupVisibleCloudOnlyRows(CloudOnlyState.Loading))
        assertNull(cloudBackupVisibleCloudOnlyRows(CloudOnlyState.Loaded(emptyList(), incomplete = false)))
        assertNull(
            cloudBackupVisibleCloudOnlyRows(
                CloudOnlyState.Failed(error = "still syncing"),
            ),
        )

        val emptyIncomplete = cloudBackupVisibleCloudOnlyRows(CloudOnlyState.Loaded(emptyList(), incomplete = true))
        assertEquals(emptyList<CloudBackupWalletItem>(), emptyIncomplete?.notOnDevice)
        assertEquals(true, emptyIncomplete?.showsNotOnDevice(CloudBackupRestoreAllState.NotShown))
    }

    @Test
    fun cloudOnlyCopiesOfLocalWalletsListApartFromNotOnDevice() {
        val extraCopy =
            cloudBackupWallet().copy(
                recordId = "copy-record",
                syncStatus = CloudBackupWalletStatus.ALREADY_ON_DEVICE,
            )

        val rows =
            cloudBackupVisibleCloudOnlyRows(
                CloudOnlyState.Loaded(listOf(cloudBackupWallet(), extraCopy), incomplete = false),
            )
        assertEquals(listOf(cloudBackupWallet()), rows?.notOnDevice)
        assertEquals(listOf(extraCopy), rows?.extraCopies)

        val onlyCopies =
            cloudBackupVisibleCloudOnlyRows(CloudOnlyState.Loaded(listOf(extraCopy), incomplete = false))
        assertEquals(false, onlyCopies?.showsNotOnDevice(CloudBackupRestoreAllState.NotShown))
        assertEquals(
            true,
            onlyCopies?.showsNotOnDevice(CloudBackupRestoreAllState.RetryAvailable(walletCount = 1u)),
        )
    }

    @Test
    fun otherBackupsSectionRequiresNonemptyLoadedInventory() {
        assertNull(cloudBackupVisibleOtherBackupsSummary(CloudBackupOtherBackupsState.NotChecked))
        assertNull(cloudBackupVisibleOtherBackupsSummary(CloudBackupOtherBackupsState.Checking))
        assertNull(
            cloudBackupVisibleOtherBackupsSummary(
                CloudBackupOtherBackupsState.LoadFailed(
                    reason = CloudBackupInventoryIncompleteReason.PROVIDER_SYNC_PENDING,
                ),
            ),
        )
        assertNull(
            cloudBackupVisibleOtherBackupsSummary(
                CloudBackupOtherBackupsState.Loaded(
                    summary = otherBackupsSummary(namespaceCount = 0u, walletCount = 0u),
                ),
            ),
        )
        assertEquals(
            1u,
            cloudBackupVisibleOtherBackupsSummary(
                CloudBackupOtherBackupsState.Loaded(
                    summary = otherBackupsSummary(namespaceCount = 1u, walletCount = 2u),
                ),
            )?.namespaceCount,
        )
    }

    @Test
    fun supplementalFailuresExposeContext() {
        assertEquals(
            "Drive inventory unavailable",
            cloudBackupCloudOnlyFailureMessage(
                CloudOnlyState.Failed(error = "Drive inventory unavailable"),
            ),
        )
        assertNull(
            cloudBackupCloudOnlyFailureMessage(
                CloudOnlyState.Loaded(emptyList(), incomplete = false),
            ),
        )

        val reason = CloudBackupInventoryIncompleteReason.AUTHORIZATION_REQUIRED
        assertEquals(
            reason,
            cloudBackupOtherBackupsFailureReason(
                CloudBackupOtherBackupsState.LoadFailed(reason = reason),
            ),
        )
        val message = cloudBackupOtherBackupsFailureMessage(reason)
        assertTrue(message.contains("another backup key"))
        assertTrue(message.contains("Reconnect Google Drive"))
    }

    private fun cloudBackupWallet(): CloudBackupWalletItem =
        CloudBackupWalletItem(
            name = "Savings",
            network = null,
            walletMode = null,
            walletType = null,
            fingerprint = null,
            labelCount = null,
            backupUpdatedAt = null,
            syncStatus = org.bitcoinppl.cove_core.CloudBackupWalletStatus.CONFIRMED,
            restoreFailure = null,
            recordId = "wallet-record",
        )

    private fun otherBackupsSummary(
        namespaceCount: UInt,
        walletCount: UInt,
    ): CloudBackupOtherBackupsSummary =
        CloudBackupOtherBackupsSummary(
            namespaceCount = namespaceCount,
            walletCount = walletCount,
            passkeyHints = emptyList(),
        )
}
