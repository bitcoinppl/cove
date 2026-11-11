package org.bitcoinppl.cove.flows.cloudbackup

import org.bitcoinppl.cove.cloudbackup.CloudBackupManager
import org.bitcoinppl.cove.cloudbackup.configured
import org.bitcoinppl.cove.cloudbackup.lookup
import org.bitcoinppl.cove.cloudbackup.section
import org.bitcoinppl.cove.cloudbackup.verification
import org.bitcoinppl.cove_core.CloudBackupActionAvailability
import org.bitcoinppl.cove_core.CloudBackupConfiguredScreen
import org.bitcoinppl.cove_core.CloudBackupDisableScreen
import org.bitcoinppl.cove_core.CloudBackupInventoryIncompleteReason
import org.bitcoinppl.cove_core.CloudBackupListedRow
import org.bitcoinppl.cove_core.CloudBackupProgressSection
import org.bitcoinppl.cove_core.CloudBackupRestoreAllState
import org.bitcoinppl.cove_core.CloudBackupScreenAction
import org.bitcoinppl.cove_core.CloudBackupScreenBody
import org.bitcoinppl.cove_core.CloudBackupSection
import org.bitcoinppl.cove_core.CloudBackupSettingsRowStatus
import org.bitcoinppl.cove_core.CloudBackupSyncState
import org.bitcoinppl.cove_core.CloudBackupVerificationState
import org.bitcoinppl.cove_core.CloudOnlyOperation

// Read-only probes over the Rust-owned screen, so full-launch tests wait on what the user sees
// instead of on manager flags that no longer exist

internal val CloudBackupManager.configured: CloudBackupConfiguredScreen?
    get() = screen.configured

internal val CloudBackupManager.isConfigured: Boolean
    get() = screen.body is CloudBackupScreenBody.Configured

internal val CloudBackupManager.isDisabled: Boolean
    get() = screen.body is CloudBackupScreenBody.Disabled

internal val CloudBackupManager.progressSection: CloudBackupProgressSection?
    get() = configured?.section<CloudBackupSection.Progress>()?.v1

// a configured detail whose inventory is neither being checked nor reported incomplete
internal val CloudBackupManager.isInventoryComplete: Boolean
    get() {
        val configured = configured ?: return false

        return when (configured.section<CloudBackupSection.Progress>()?.v1) {
            is CloudBackupProgressSection.InventoryChecking,
            is CloudBackupProgressSection.InventoryIncomplete,
            -> false
            else -> true
        }
    }

internal val CloudBackupManager.isInventoryChecking: Boolean
    get() = progressSection is CloudBackupProgressSection.InventoryChecking

internal val CloudBackupManager.isInventoryIncomplete: Boolean
    get() = progressSection is CloudBackupProgressSection.InventoryIncomplete

internal val CloudBackupManager.isInventoryOffline: Boolean
    get() =
        (progressSection as? CloudBackupProgressSection.InventoryIncomplete)?.reason ==
            CloudBackupInventoryIncompleteReason.OFFLINE

internal val CloudBackupManager.verificationState: CloudBackupVerificationState?
    get() = screen.verification?.state

internal val CloudBackupManager.hasPendingUploadVerification: Boolean
    get() = verificationState is CloudBackupVerificationState.AwaitingUploadConfirmation

internal val CloudBackupManager.isTurningOff: Boolean
    get() =
        screen.settingsRow is CloudBackupSettingsRowStatus.Disabling ||
            configured?.disable is CloudBackupDisableScreen.Draining

internal val CloudBackupManager.walletsSection
    get() = configured?.section<CloudBackupSection.Wallets>()?.v1

internal val CloudBackupManager.notOnDeviceSection
    get() = configured?.section<CloudBackupSection.NotOnDevice>()?.v1

// a screen without the section shows no Restore All, which is how NotShown reads
internal val CloudBackupManager.restoreAllState: CloudBackupRestoreAllState
    get() = notOnDeviceSection?.restoreAll ?: CloudBackupRestoreAllState.NotShown

internal val CloudBackupManager.cloudOnlyOperation: CloudOnlyOperation
    get() = notOnDeviceSection?.operation ?: CloudOnlyOperation.Idle

internal val CloudBackupManager.syncState: CloudBackupSyncState
    get() = walletsSection?.sync ?: CloudBackupSyncState.Idle

internal val CloudBackupManager.notOnDeviceRows: List<CloudBackupListedRow>
    get() = notOnDeviceSection?.rows.orEmpty()

internal val CloudBackupManager.extraCopyRows: List<CloudBackupListedRow>
    get() = configured?.section<CloudBackupSection.ExtraCopies>()?.v1?.rows.orEmpty()

internal val CloudBackupManager.otherBackupsSection
    get() = configured?.section<CloudBackupSection.OtherBackups>()?.v1

internal fun CloudBackupManager.availability(action: CloudBackupScreenAction): CloudBackupActionAvailability? =
    screen.actions.lookup(action)

internal fun CloudBackupManager.isActionAvailable(action: CloudBackupScreenAction): Boolean =
    availability(action) is CloudBackupActionAvailability.Available

internal fun CloudBackupManager.describeScreen(): String =
    "body=${screen.body::class.simpleName}, row=${screen.settingsRow}, " +
        "progress=$progressSection, verification=$verificationState, outcome=${screen.lastOutcome}"
