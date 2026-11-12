package org.bitcoinppl.cove.cloudbackup

import org.bitcoinppl.cove_core.CloudBackupOperationKind
import org.bitcoinppl.cove_core.CloudBackupRejection
import org.junit.Assert.assertEquals
import org.junit.Test

class CloudBackupScreenCopyTest {
    // the names are split into two groups, so a kind in the wrong group throws here instead of in
    // front of a person
    @Test
    fun everyOperationKindIsNamedInItsWaitingCaption() {
        val captions =
            CloudBackupOperationKind.entries.associateWith { kind ->
                CloudBackupRejection.OperationRunning(kind).caption()
            }

        assertEquals(
            "Waiting for the wallet restore to finish.",
            captions[CloudBackupOperationKind.RESTORE_CLOUD_WALLET],
        )
        assertEquals("Waiting for Turn Off to finish.", captions[CloudBackupOperationKind.RESUME_TURN_OFF])
        assertEquals(
            "Waiting for the Google account switch to finish.",
            captions[CloudBackupOperationKind.DRIVE_ACCOUNT_SWITCH],
        )

        // only Turn Off and its resume share a name
        assertEquals(CloudBackupOperationKind.entries.size - 1, captions.values.distinct().size)
    }

    // Enable while on and work before setup finishes have their own reasons, each worded as itself
    @Test
    fun alreadyOnAndSetupNotFinishedAreWordedApart() {
        assertEquals("Cloud Backup is already on.", CloudBackupRejection.AlreadyOn.caption())
        assertEquals("Cloud Backup isn't set up yet.", CloudBackupRejection.SetupNotFinished.caption())
        assertEquals("Cloud Backup is off.", CloudBackupRejection.CloudBackupOff.caption())
    }
}
