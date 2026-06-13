package org.bitcoinppl.cove.cloudbackup

import org.bitcoinppl.cove_core.device.CloudSyncHealth
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotEquals
import org.junit.Test

class CloudBackupWalletRowsHelpersTest {
    @Test
    fun headerTitleUsesActiveOnlyForConfirmedUploads() {
        assertEquals("Cloud Backup Active", cloudBackupHeaderTitle(CloudSyncHealth.AllUploaded))

        val unhealthyStates =
            listOf(
                CloudSyncHealth.Unknown,
                CloudSyncHealth.Uploading,
                CloudSyncHealth.NoFiles,
                CloudSyncHealth.AuthorizationRequired("wrong account"),
                CloudSyncHealth.Unavailable,
                CloudSyncHealth.Failed("drive unavailable"),
            )

        unhealthyStates.forEach { health ->
            assertNotEquals("Cloud Backup Active", cloudBackupHeaderTitle(health))
        }
    }
}
