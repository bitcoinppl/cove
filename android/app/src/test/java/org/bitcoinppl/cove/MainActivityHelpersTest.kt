package org.bitcoinppl.cove

import org.bitcoinppl.cove_core.CatastrophicCloudRestoreResult
import org.junit.Assert.assertEquals
import org.junit.Test

class MainActivityHelpersTest {
    @Test
    fun catastrophicRestoreBackupFoundHasNoFailureMessage() {
        assertEquals(null, CatastrophicCloudRestoreResult.BackupFound.failureMessage)
    }

    @Test
    fun catastrophicRestoreAuthorizationErrorIsUserVisible() {
        val message = CatastrophicCloudRestoreResult.Inconclusive("wrong google drive account")
            .failureMessage

        assertEquals("wrong google drive account", message)
    }

    @Test
    fun catastrophicRestoreUnreadableBackupErrorIsUserVisible() {
        val message = CatastrophicCloudRestoreResult.Unreadable(
            "Cloud Backup data could not be read: master key backup is unreadable",
        ).failureMessage

        assertEquals("Cloud Backup data could not be read: master key backup is unreadable", message)
    }
}
