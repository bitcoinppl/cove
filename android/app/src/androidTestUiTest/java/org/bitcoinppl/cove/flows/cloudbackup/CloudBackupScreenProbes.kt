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
import org.bitcoinppl.cove_core.CloudBackupRootPrompt
import org.bitcoinppl.cove_core.CloudBackupScreenAction
import org.bitcoinppl.cove_core.CloudBackupScreenBody
import org.bitcoinppl.cove_core.CloudBackupSection
import org.bitcoinppl.cove_core.CloudBackupSettingsRowStatus
import org.bitcoinppl.cove_core.CloudBackupSyncState
import org.bitcoinppl.cove_core.CloudBackupVerificationState
import org.bitcoinppl.cove_core.CloudOnlyOperation
import org.junit.Assert.assertTrue
import kotlin.reflect.KClass

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

/**
 * Fails unless the screen raises no root prompt, which the contract says with
 * [CloudBackupRootPrompt.None], never null
 */
internal fun CloudBackupManager.assertNoRootPrompt(message: String) {
    val prompt = screen.rootPrompt
    assertTrue("$message: $prompt", prompt is CloudBackupRootPrompt.None)
}

internal fun CloudBackupManager.describeScreen(): String =
    "body=${screen.body::class.simpleName}, row=${screen.settingsRow}, " +
        "progress=$progressSection, verification=$verificationState, outcome=${screen.lastOutcome}, " +
        "disable=${configured?.disable}"

/**
 * Waits until the screen's body is a [body], Rust offers every action in [available] as available
 * and none in [unavailable], failing with [message] and the screen if it never does
 *
 * Checks what the user may do next as Rust decided it, rather than what the screen happens to
 * draw. Actions named in neither set aren't checked, so a test names only the ones its scenario is
 * about, and an action in [unavailable] may be either absent or unavailable
 */
internal fun CloudBackupManager.assertNextActions(
    message: String,
    body: KClass<out CloudBackupScreenBody> = CloudBackupScreenBody.Configured::class,
    available: Set<CloudBackupScreenAction> = emptySet(),
    unavailable: Set<CloudBackupScreenAction> = emptySet(),
    timeoutMs: Long = 30_000L,
) {
    val deadline = System.currentTimeMillis() + timeoutMs

    while (System.currentTimeMillis() < deadline) {
        if (offersNextActions(body, available, unavailable)) return

        Thread.sleep(100)
    }

    assertTrue(
        "$message; wanted body=${body.simpleName}, available=$available, unavailable=$unavailable; " +
            "${describeScreen()}, actions=${describeActions()}",
        offersNextActions(body, available, unavailable),
    )
}

private fun CloudBackupManager.offersNextActions(
    body: KClass<out CloudBackupScreenBody>,
    available: Set<CloudBackupScreenAction>,
    unavailable: Set<CloudBackupScreenAction>,
): Boolean {
    val screen = screen
    val offered =
        screen.actions.entries
            .filter { it.availability is CloudBackupActionAvailability.Available }
            .map { it.action }
            .toSet()

    return body.isInstance(screen.body) && offered.containsAll(available) && offered.none(unavailable::contains)
}

internal fun CloudBackupManager.describeActions(): String =
    screen.actions.entries.joinToString(prefix = "[", postfix = "]") { entry ->
        "${entry.action}=${entry.availability::class.simpleName}"
    }
