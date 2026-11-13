package org.bitcoinppl.cove.cloudbackup

import androidx.compose.foundation.layout.size
import androidx.compose.runtime.getValue
import androidx.compose.runtime.setValue
import org.bitcoinppl.cove_core.CloudBackupDisableConfirmation
import org.bitcoinppl.cove_core.CloudBackupDisableImpact
import org.bitcoinppl.cove_core.CloudBackupInventoryAssessment
import org.bitcoinppl.cove_core.CloudBackupNamedCount

/** The words of the Turn Off review; Rust decides the counts, names and how many steps */
internal data class CloudBackupTurnOffReviewCopy(
    val risks: List<String>,
    val finalTitle: String,
    val finalLines: List<String>,
)

/**
 * Builds the review copy from the impact, one line per shape that applies
 *
 * Never says "all": other-passkey backups stay, and the final step names them as untouched
 */
internal fun cloudBackupTurnOffReviewCopy(impact: CloudBackupDisableImpact): CloudBackupTurnOffReviewCopy {
    val finalTitle =
        when (impact.confirmation) {
            CloudBackupDisableConfirmation.ONE -> "Turn Off Cloud Backup?"
            CloudBackupDisableConfirmation.TWO -> "Delete These Backups and Turn Off Cloud Backup?"
        }

    return CloudBackupTurnOffReviewCopy(
        risks = turnOffReviewRisks(impact),
        finalTitle = finalTitle,
        finalLines = turnOffReviewFinalLines(impact),
    )
}

/** What Turn Off could lose, in the order the first step lists it */
private fun turnOffReviewRisks(impact: CloudBackupDisableImpact): List<String> =
    buildList {
        if (impact.assessment !is CloudBackupInventoryAssessment.Complete) {
            add(
                "Cove couldn't check what's in Cloud Backup. " +
                    "Wallets that aren't on this device may be deleted.",
            )
        }

        if (impact.notOnDevice.isNotEmpty()) {
            add(
                "These wallets are only in Cloud Backup: ${impact.notOnDevice.listed()}. " +
                    "Turning off Cloud Backup deletes them. Without their recovery words, " +
                    "they can't be recovered.",
            )
        }

        if (impact.ownSecretMissingLocally.isNotEmpty()) {
            add(
                "This device no longer has what it needs to restore these wallets: " +
                    "${impact.ownSecretMissingLocally.listed()}. Turning off Cloud Backup " +
                    "deletes their backups.",
            )
        }

        if (impact.unchecked > 0u) {
            add(
                "Cove can't check ${backups(impact.unchecked)} without your passkey. " +
                    "They may be wallets that aren't on this device.",
            )
        }

        val unreadable = impact.unreadable + impact.unsupported
        if (unreadable > 0u) {
            add(
                "Cove can't read ${backups(unreadable)}. They may be damaged, or made by a newer " +
                    "version of Cove. They will be deleted too.",
            )
        }
    }

/** What the final step deletes and what it leaves */
private fun turnOffReviewFinalLines(impact: CloudBackupDisableImpact): List<String> =
    buildList {
        if (impact.confirmation == CloudBackupDisableConfirmation.ONE) {
            add(
                "This deletes the backups of your ${wallets(impact.ownBackups)} from $PROVIDER. " +
                    "Your wallets stay on this device and will no longer be backed up.",
            )
        } else {
            add("This can't be undone.")
        }

        if (impact.extraCopies > 0u) {
            add(
                "This includes ${count(impact.extraCopies, "extra copy", "extra copies")}, " +
                    "and any labels saved only in them.",
            )
        }

        if (impact.otherBackups > 0u) {
            add(
                "${backups(impact.otherBackups).replaceFirstChar(Char::uppercaseChar)} made with another " +
                    "passkey won't be deleted. To remove them later, turn Cloud Backup on again, " +
                    "or delete them from $PROVIDER storage settings.",
            )
        }
    }

internal fun CloudBackupNamedCount.isNotEmpty(): Boolean = names.isNotEmpty() || more > 0u

private fun CloudBackupNamedCount.listed(): String {
    val parts = names + listOfNotNull(more.takeIf { it > 0u }?.let { "$it more" })
    if (parts.size <= 1) return parts.joinToString("")

    return parts.dropLast(1).joinToString(", ") + " and " + parts.last()
}

private fun count(
    value: UInt,
    one: String,
    many: String,
): String = if (value == 1u) "1 $one" else "$value $many"

private fun backups(value: UInt): String = count(value, "backup", "backups")

private fun wallets(value: UInt): String = count(value, "wallet", "wallets")
