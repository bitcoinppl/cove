package org.bitcoinppl.cove.cloudbackup

import org.bitcoinppl.cove_core.CloudBackupActionAvailability
import org.bitcoinppl.cove_core.CloudBackupDisableNotStartedReason
import org.bitcoinppl.cove_core.CloudBackupOperationKind
import org.bitcoinppl.cove_core.CloudBackupOutcome
import org.bitcoinppl.cove_core.CloudBackupRejection
import org.bitcoinppl.cove_core.CloudBackupRowAction

/** The reason to show under a disabled control, or null when it is enabled */
internal val CloudBackupActionAvailability.caption: String?
    get() = (this as? CloudBackupActionAvailability.Unavailable)?.reason?.caption()

/** The subtitle under a control: Rust's reason while it is disabled, otherwise the control's own */
internal fun CloudBackupActionAvailability.subtitle(enabled: String?): String? =
    when (this) {
        CloudBackupActionAvailability.Available -> enabled
        is CloudBackupActionAvailability.Unavailable -> reason.caption()
    }

/** One shared reason when every action on these rows is unavailable for it, for a section header */
internal fun List<CloudBackupRowAction>.sharedUnavailableCaption(): String? {
    val reasons =
        map { action ->
            (action.availability as? CloudBackupActionAvailability.Unavailable)?.reason ?: return null
        }.distinct()

    return reasons.singleOrNull()?.caption()
}

/** The provider Android backs up to, as Cloud Backup's copy names it */
internal const val PROVIDER = "Google Drive"

/** Copy for each admission rejection; Rust decides the reason and Android only words it */
internal fun CloudBackupRejection.caption(): String =
    when (this) {
        CloudBackupRejection.CloudBackupOff,
        CloudBackupRejection.AlreadyOn,
        CloudBackupRejection.SetupNotFinished,
        -> onOffCaption()
        CloudBackupRejection.LoadFailed -> "Cove couldn't load Cloud Backup."
        CloudBackupRejection.TurnOffCommitted -> "Cloud Backup is being turned off."
        is CloudBackupRejection.OperationRunning -> "Waiting for ${operation.displayName()} to finish."
        CloudBackupRejection.InventoryNotLoaded -> "Waiting for Cove to finish checking $PROVIDER."
        CloudBackupRejection.OtherBackupsNotLoaded -> "Waiting for Cove to finish checking other backups."
        CloudBackupRejection.RestoreAllUnavailable -> "Restore All isn't available right now."
        CloudBackupRejection.NothingToCancel -> "There's nothing to cancel."
        CloudBackupRejection.NothingToContinue -> "There's nothing to continue."
        CloudBackupRejection.ReviewStale -> "Cloud Backup changed since you checked. Review it again."
        CloudBackupRejection.Resetting -> "Cove is resetting Cloud Backup on this device."
        CloudBackupRejection.TryAgainFirst -> "Try again first."
    }

/**
 * Copy for the rejections that say whether Cloud Backup is on, split out so [caption] stays under
 * detekt's complexity limit
 */
private fun CloudBackupRejection.onOffCaption(): String =
    when (this) {
        CloudBackupRejection.CloudBackupOff -> "Cloud Backup is off."
        CloudBackupRejection.AlreadyOn -> "Cloud Backup is already on."
        CloudBackupRejection.SetupNotFinished -> "Cloud Backup isn't set up yet."
        else -> error("$this doesn't say whether Cloud Backup is on")
    }

/** Names an operation for "Waiting for ... to finish." */
internal fun CloudBackupOperationKind.displayName(): String =
    when (group) {
        CloudBackupOperationGroup.CLOUD_BACKUP -> cloudBackupOperationName()
        CloudBackupOperationGroup.BACKUPS -> backupOperationName()
    }

/**
 * Splits the operation names in two, so each group's names fit one when under detekt's
 * complexity limit
 *
 * Each group's when also lists the other group's kinds, which group never sends there, so a kind
 * placed in the wrong group fails loudly; CloudBackupScreenCopyTest names every kind
 */
private enum class CloudBackupOperationGroup { CLOUD_BACKUP, BACKUPS }

private val CloudBackupOperationKind.group: CloudBackupOperationGroup
    get() =
        when (this) {
            CloudBackupOperationKind.RESTORE,
            CloudBackupOperationKind.RESTORE_CLOUD_WALLET,
            CloudBackupOperationKind.RESTORE_ALL,
            CloudBackupOperationKind.DELETE_RECORDS,
            CloudBackupOperationKind.RECOVER_OTHER_BACKUPS,
            CloudBackupOperationKind.DELETE_OTHER_BACKUPS,
            -> CloudBackupOperationGroup.BACKUPS
            CloudBackupOperationKind.ENABLE,
            CloudBackupOperationKind.TURN_OFF,
            CloudBackupOperationKind.RESUME_TURN_OFF,
            CloudBackupOperationKind.VERIFY,
            CloudBackupOperationKind.REPAIR_PASSKEY,
            CloudBackupOperationKind.REINITIALIZE,
            CloudBackupOperationKind.RECREATE_MANIFEST,
            CloudBackupOperationKind.DISCARD_PENDING_ENABLE,
            CloudBackupOperationKind.DRIVE_ACCOUNT_SWITCH,
            -> CloudBackupOperationGroup.CLOUD_BACKUP
        }

private fun CloudBackupOperationKind.cloudBackupOperationName(): String =
    when (this) {
        CloudBackupOperationKind.ENABLE -> "Cloud Backup to turn on"
        CloudBackupOperationKind.TURN_OFF,
        CloudBackupOperationKind.RESUME_TURN_OFF,
        -> "Turn Off"
        CloudBackupOperationKind.VERIFY -> "verification"
        CloudBackupOperationKind.REPAIR_PASSKEY -> "the passkey repair"
        CloudBackupOperationKind.REINITIALIZE -> "Cloud Backup to reinitialize"
        CloudBackupOperationKind.RECREATE_MANIFEST -> "the backup index to be recreated"
        CloudBackupOperationKind.DISCARD_PENDING_ENABLE -> "the incomplete setup to be removed"
        CloudBackupOperationKind.DRIVE_ACCOUNT_SWITCH -> "the Google account switch"
        CloudBackupOperationKind.RESTORE,
        CloudBackupOperationKind.RESTORE_CLOUD_WALLET,
        CloudBackupOperationKind.RESTORE_ALL,
        CloudBackupOperationKind.DELETE_RECORDS,
        CloudBackupOperationKind.RECOVER_OTHER_BACKUPS,
        CloudBackupOperationKind.DELETE_OTHER_BACKUPS,
        -> error("$this is in the backups group")
    }

private fun CloudBackupOperationKind.backupOperationName(): String =
    when (this) {
        CloudBackupOperationKind.RESTORE -> "the restore"
        CloudBackupOperationKind.RESTORE_CLOUD_WALLET -> "the wallet restore"
        CloudBackupOperationKind.RESTORE_ALL -> "Restore All"
        CloudBackupOperationKind.DELETE_RECORDS -> "the delete"
        CloudBackupOperationKind.RECOVER_OTHER_BACKUPS -> "the other backups to be recovered"
        CloudBackupOperationKind.DELETE_OTHER_BACKUPS -> "the other backups to be deleted"
        CloudBackupOperationKind.ENABLE,
        CloudBackupOperationKind.TURN_OFF,
        CloudBackupOperationKind.RESUME_TURN_OFF,
        CloudBackupOperationKind.VERIFY,
        CloudBackupOperationKind.REPAIR_PASSKEY,
        CloudBackupOperationKind.REINITIALIZE,
        CloudBackupOperationKind.RECREATE_MANIFEST,
        CloudBackupOperationKind.DISCARD_PENDING_ENABLE,
        CloudBackupOperationKind.DRIVE_ACCOUNT_SWITCH,
        -> error("$this is in the Cloud Backup group")
    }

private fun CloudBackupDisableNotStartedReason.caption(): String =
    when (this) {
        CloudBackupDisableNotStartedReason.OFFLINE ->
            "Cloud Backup is still on. Cove couldn't reach $PROVIDER."
        CloudBackupDisableNotStartedReason.AUTHORIZATION_REQUIRED ->
            "Cloud Backup is still on. Cove needs access to $PROVIDER."
        CloudBackupDisableNotStartedReason.PROVIDER_UNAVAILABLE ->
            "Cloud Backup is still on. $PROVIDER isn't available right now."
        CloudBackupDisableNotStartedReason.WRITES_DID_NOT_DRAIN ->
            "Cloud Backup is still on. Cove couldn't finish the backups in progress."
        CloudBackupDisableNotStartedReason.SAVE_FAILED ->
            "Cloud Backup is still on. Cove couldn't save the change."
    }

internal fun CloudBackupOutcome.message(): String =
    when (this) {
        is CloudBackupOutcome.Rejected -> reason.caption()
        is CloudBackupOutcome.DisableNotStarted -> reason.caption()
        CloudBackupOutcome.PasskeyNotCompleted -> "The passkey request didn't finish. Nothing changed."
        is CloudBackupOutcome.Failed -> message
    }
