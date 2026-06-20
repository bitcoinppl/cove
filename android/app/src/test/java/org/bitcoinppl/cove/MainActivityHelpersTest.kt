package org.bitcoinppl.cove

import org.bitcoinppl.cove_core.CatastrophicCloudRestoreInconclusiveReason
import org.bitcoinppl.cove_core.CatastrophicCloudRestoreProvider
import org.bitcoinppl.cove_core.CatastrophicCloudRestoreResult
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

class MainActivityHelpersTest {
    @Test
    fun catastrophicRestoreBackupFoundHasNoFailureMessage() {
        assertNull(CatastrophicCloudRestoreResult.BackupFound.localizedFailureMessage())
    }

    @Test
    fun catastrophicRestoreAuthorizationRequiredMessageNamesProvider() {
        val message =
            CatastrophicCloudRestoreResult.Inconclusive(
                provider = CatastrophicCloudRestoreProvider.GOOGLE_DRIVE,
                reason = CatastrophicCloudRestoreInconclusiveReason.AUTHORIZATION_REQUIRED,
            ).localizedFailureMessage()

        assertEquals(
            UiText.resource(
                R.string.common_remaining_cloud_backup_authorization_required,
                UiText.resource(R.string.common_remaining_google_drive),
            ),
            message,
        )
    }

    @Test
    fun catastrophicRestoreUnreadableBackupErrorUsesLocalizedMessage() {
        val message = CatastrophicCloudRestoreResult.Unreadable.localizedFailureMessage()

        assertEquals(
            UiText.resource(R.string.common_remaining_cloud_backup_unreadable),
            message,
        )
    }
}
