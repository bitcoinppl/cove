package org.bitcoinppl.cove

import org.bitcoinppl.cove_core.device.CloudStorageException
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class MainActivityHelpersTest {
    @Test
    fun catastrophicRestoreCheckRequiresFoundBackupBeforeConfirmation() {
        assertTrue(catastrophicCloudRestoreCheckResult(0) is CatastrophicCloudRestoreCheck.Failed)
        assertEquals(
            CatastrophicCloudRestoreCheck.BackupFound(namespaceCount = 2),
            catastrophicCloudRestoreCheckResult(2),
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
}
