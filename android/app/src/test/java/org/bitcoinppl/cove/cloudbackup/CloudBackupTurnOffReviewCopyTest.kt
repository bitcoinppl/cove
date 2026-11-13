package org.bitcoinppl.cove.cloudbackup

import org.bitcoinppl.cove_core.CloudBackupDisableConfirmation
import org.bitcoinppl.cove_core.CloudBackupDisableImpact
import org.bitcoinppl.cove_core.CloudBackupInventoryAssessment
import org.bitcoinppl.cove_core.CloudBackupInventoryIncompleteReason
import org.bitcoinppl.cove_core.CloudBackupNamedCount
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Test

class CloudBackupTurnOffReviewCopyTest {
    @Test
    fun ownBackupsOnlyNeedOneConfirmationThatNamesTheCount() {
        val copy = cloudBackupTurnOffReviewCopy(impact(ownBackups = 3u))

        assertEquals(emptyList<String>(), copy.risks)
        assertEquals("Turn Off Cloud Backup?", copy.finalTitle)
        assertEquals(
            listOf(
                "This deletes the backups of your 3 wallets from Google Drive. " +
                    "Your wallets stay on this device and will no longer be backed up.",
            ),
            copy.finalLines,
        )
    }

    @Test
    fun walletsOnlyInCloudBackupAreNamedWithTheRestCounted() {
        val copy =
            cloudBackupTurnOffReviewCopy(
                impact(
                    notOnDevice = CloudBackupNamedCount(names = listOf("Savings", "Travel"), more = 2u),
                    unchecked = 1u,
                    unreadable = 1u,
                    unsupported = 1u,
                    confirmation = CloudBackupDisableConfirmation.TWO,
                ),
            )

        assertEquals(
            listOf(
                "These wallets are only in Cloud Backup: Savings, Travel and 2 more. Turning off Cloud " +
                    "Backup deletes them. Without their recovery words, they can't be recovered.",
                "Cove can't check 1 backup without your passkey. They may be wallets that aren't on " +
                    "this device.",
                "Cove can't read 2 backups. They may be damaged, or made by a newer version of Cove. " +
                    "They will be deleted too.",
            ),
            copy.risks,
        )
        assertEquals("Delete These Backups and Turn Off Cloud Backup?", copy.finalTitle)
        assertEquals("This can't be undone.", copy.finalLines.first())
    }

    @Test
    fun incompleteInventoryWarnsBeforeAnyCountsAndNeverClaimsAll() {
        val copy =
            cloudBackupTurnOffReviewCopy(
                impact(
                    assessment = CloudBackupInventoryAssessment.Partial(CloudBackupInventoryIncompleteReason.OFFLINE),
                    ownSecretMissingLocally = CloudBackupNamedCount(names = listOf("Cold Storage"), more = 0u),
                    confirmation = CloudBackupDisableConfirmation.TWO,
                ),
            )

        assertEquals(
            listOf(
                "Cove couldn't check what's in Cloud Backup. Wallets that aren't on this device may be " +
                    "deleted.",
                "This device no longer has what it needs to restore these wallets: Cold Storage. " +
                    "Turning off Cloud Backup deletes their backups.",
            ),
            copy.risks,
        )
        assertFalse(
            "the review must not claim it deletes all backups when it couldn't check them",
            (copy.risks + copy.finalLines).any { " all " in it },
        )
    }

    @Test
    fun extraCopiesAndOtherPasskeyBackupsAreExplainedInTheFinalStep() {
        val copy = cloudBackupTurnOffReviewCopy(impact(ownBackups = 1u, extraCopies = 2u, otherBackups = 1u))

        assertEquals(
            listOf(
                "This deletes the backups of your 1 wallet from Google Drive. " +
                    "Your wallets stay on this device and will no longer be backed up.",
                "This includes 2 extra copies, and any labels saved only in them.",
                "1 backup made with another passkey won't be deleted. To remove them later, turn Cloud " +
                    "Backup on again, or delete them from Google Drive storage settings.",
            ),
            copy.finalLines,
        )
    }

    private fun impact(
        assessment: CloudBackupInventoryAssessment = CloudBackupInventoryAssessment.Complete,
        ownBackups: UInt = 0u,
        notOnDevice: CloudBackupNamedCount = NONE,
        ownSecretMissingLocally: CloudBackupNamedCount = NONE,
        extraCopies: UInt = 0u,
        unchecked: UInt = 0u,
        unreadable: UInt = 0u,
        unsupported: UInt = 0u,
        otherBackups: UInt = 0u,
        confirmation: CloudBackupDisableConfirmation = CloudBackupDisableConfirmation.ONE,
    ) = CloudBackupDisableImpact(
        assessment = assessment,
        ownBackups = ownBackups,
        notOnDevice = notOnDevice,
        ownSecretMissingLocally = ownSecretMissingLocally,
        extraCopies = extraCopies,
        unchecked = unchecked,
        unreadable = unreadable,
        unsupported = unsupported,
        otherBackups = otherBackups,
        confirmation = confirmation,
    )

    private companion object {
        val NONE = CloudBackupNamedCount(names = emptyList(), more = 0u)
    }
}
