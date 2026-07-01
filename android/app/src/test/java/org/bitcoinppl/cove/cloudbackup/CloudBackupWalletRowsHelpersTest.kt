package org.bitcoinppl.cove.cloudbackup

import org.bitcoinppl.cove.R
import org.bitcoinppl.cove.UiText
import org.bitcoinppl.cove_core.device.CloudSyncHealth
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotEquals
import org.junit.Test

class CloudBackupWalletRowsHelpersTest {
    @Test
    fun headerTitleUsesActiveOnlyForConfirmedUploads() {
        assertEquals(
            UiText.resource(R.string.cloud_backup_header_active),
            cloudBackupHeaderTitleText(CloudSyncHealth.ALL_UPLOADED),
        )

        val unhealthyStates =
            listOf(
                CloudSyncHealth.UNKNOWN,
                CloudSyncHealth.UPLOADING,
                CloudSyncHealth.NO_FILES,
                CloudSyncHealth.AUTHORIZATION_REQUIRED,
                CloudSyncHealth.UNAVAILABLE,
                CloudSyncHealth.FAILED,
            )

        unhealthyStates.forEach { health ->
            assertNotEquals(
                UiText.resource(R.string.cloud_backup_header_active),
                cloudBackupHeaderTitleText(health),
            )
        }
    }
}
