package org.bitcoinppl.cove

import org.bitcoinppl.cove_core.device.CloudStorageException
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class MainActivityHelpersTest {
    @Test
    fun catastrophicRestoreCheckRequiresFoundBackupBeforeConfirmation() {
        assertTrue(catastrophicCloudRestoreCheckResult(false) is CatastrophicCloudRestoreCheck.Failed)
        assertEquals(
            CatastrophicCloudRestoreCheck.BackupFound,
            catastrophicCloudRestoreCheckResult(true),
        )
    }

    @Test
    fun catastrophicRestoreAuthorizationErrorIsUserVisible() {
        val message =
            catastrophicCloudRestoreErrorMessage(
                CloudStorageException.AuthorizationRequired("wrong google drive account"),
            )

        assertEquals("wrong google drive account", message)
    }

    @Test
    fun catastrophicRestoreUnreadableBackupErrorIsUserVisible() {
        val message =
            catastrophicCloudRestoreErrorMessage(
                CloudStorageException.DownloadFailed("master key backup is unreadable"),
            )

        assertEquals("Cloud Backup data could not be read: master key backup is unreadable", message)
    }
}
