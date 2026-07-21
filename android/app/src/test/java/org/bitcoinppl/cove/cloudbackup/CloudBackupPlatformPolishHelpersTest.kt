package org.bitcoinppl.cove.cloudbackup

import org.bitcoinppl.cove.flows.SettingsFlow.CloudBackupSettingsSeverity
import org.bitcoinppl.cove.flows.SettingsFlow.cloudBackupSettingsSeverity
import org.bitcoinppl.cove_core.CloudBackupProgress
import org.bitcoinppl.cove_core.CloudBackupSettingsRowStatus
import org.bitcoinppl.cove_core.CloudBackupSyncState
import org.bitcoinppl.cove_core.CloudBackupWalletItem
import org.bitcoinppl.cove_core.CloudBackupWalletRestoreFailure
import org.bitcoinppl.cove_core.CloudBackupWalletStatus
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class CloudBackupPlatformPolishHelpersTest {
    @Test
    fun knownEnableProgressProducesBoundedDeterminateFraction() {
        assertEquals(
            0.4f,
            cloudBackupProgressFraction(CloudBackupProgress(completed = 2u, total = 5u)),
        )
        assertEquals(
            1f,
            cloudBackupProgressFraction(CloudBackupProgress(completed = 6u, total = 5u)),
        )
        assertNull(cloudBackupProgressFraction(CloudBackupProgress(completed = 0u, total = 0u)))
        assertNull(cloudBackupProgressFraction(null))
    }

    @Test
    fun blockedPendingConfirmationAndFailedSyncExposeRecoveryActions() {
        assertEquals("Reconnect Google Drive", pendingUploadConfirmationActionTitle(true))
        assertNull(pendingUploadConfirmationActionTitle(false))

        assertTrue(
            shouldShowCloudBackupSyncAction(
                hasNeedsSync = false,
                syncState = CloudBackupSyncState.Failed("upload failed"),
            ),
        )
        assertTrue(
            shouldShowCloudBackupSyncAction(
                hasNeedsSync = true,
                syncState = CloudBackupSyncState.Idle,
            ),
        )
        assertFalse(
            shouldShowCloudBackupSyncAction(
                hasNeedsSync = false,
                syncState = CloudBackupSyncState.Idle,
            ),
        )
    }

    @Test
    fun settingsStatusProjectsNonColorSeverity() {
        assertEquals(
            CloudBackupSettingsSeverity.NEUTRAL,
            cloudBackupSettingsSeverity(CloudBackupSettingsRowStatus.Disabled),
        )
        assertEquals(
            CloudBackupSettingsSeverity.INFO,
            cloudBackupSettingsSeverity(CloudBackupSettingsRowStatus.Confirming),
        )
        assertEquals(
            CloudBackupSettingsSeverity.SUCCESS,
            cloudBackupSettingsSeverity(CloudBackupSettingsRowStatus.Active),
        )
        assertEquals(
            CloudBackupSettingsSeverity.WARNING,
            cloudBackupSettingsSeverity(CloudBackupSettingsRowStatus.AuthorizationRequired("sign in")),
        )
        assertEquals(
            CloudBackupSettingsSeverity.WARNING,
            cloudBackupSettingsSeverity(CloudBackupSettingsRowStatus.RecoveryRequired),
        )
        assertEquals(
            CloudBackupSettingsSeverity.ERROR,
            cloudBackupSettingsSeverity(CloudBackupSettingsRowStatus.Error("upload failed")),
        )
    }

    @Test
    fun walletRowNamesAvailableActionsAndRestoreButtonState() {
        val item =
            CloudBackupWalletItem(
                recordId = "record-1",
                name = "Vacation Fund",
                walletMode = null,
                walletType = null,
                network = null,
                fingerprint = null,
                backupUpdatedAt = null,
                labelCount = null,
                syncStatus = CloudBackupWalletStatus.DELETED_FROM_DEVICE,
                restoreFailure = null,
            )

        assertEquals(
            "Show restore and delete actions for Vacation Fund",
            cloudBackupWalletRowActionLabel(item),
        )
        assertEquals(
            "Show restore and delete actions for Vacation Fund",
            cloudBackupWalletRowActionLabel(
                item.copy(
                    restoreFailure =
                        CloudBackupWalletRestoreFailure(
                            message = "Google Drive could not finish the restore",
                        ),
                ),
            ),
        )
        assertEquals("Restore to this device", cloudBackupWalletRestoreActionTitle(false))
        assertEquals("Retry restore", cloudBackupWalletRestoreActionTitle(true))
    }
}
