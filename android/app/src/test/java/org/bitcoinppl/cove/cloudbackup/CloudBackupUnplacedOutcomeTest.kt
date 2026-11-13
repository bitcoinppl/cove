package org.bitcoinppl.cove.cloudbackup

import org.bitcoinppl.cove_core.CloudBackupActionAvailability
import org.bitcoinppl.cove_core.CloudBackupActionEntry
import org.bitcoinppl.cove_core.CloudBackupActions
import org.bitcoinppl.cove_core.CloudBackupConfiguredScreen
import org.bitcoinppl.cove_core.CloudBackupDisableScreen
import org.bitcoinppl.cove_core.CloudBackupLastOutcome
import org.bitcoinppl.cove_core.CloudBackupListedRecord
import org.bitcoinppl.cove_core.CloudBackupListedRow
import org.bitcoinppl.cove_core.CloudBackupMenuControl
import org.bitcoinppl.cove_core.CloudBackupNotOnDeviceSection
import org.bitcoinppl.cove_core.CloudBackupOutcome
import org.bitcoinppl.cove_core.CloudBackupOutcomeTarget
import org.bitcoinppl.cove_core.CloudBackupRestoreAllState
import org.bitcoinppl.cove_core.CloudBackupRootPrompt
import org.bitcoinppl.cove_core.CloudBackupRowControl
import org.bitcoinppl.cove_core.CloudBackupRowTarget
import org.bitcoinppl.cove_core.CloudBackupScreen
import org.bitcoinppl.cove_core.CloudBackupScreenAction
import org.bitcoinppl.cove_core.CloudBackupScreenBody
import org.bitcoinppl.cove_core.CloudBackupSection
import org.bitcoinppl.cove_core.CloudBackupSettingsRowStatus
import org.bitcoinppl.cove_core.CloudBackupWalletItem
import org.bitcoinppl.cove_core.CloudBackupWalletStatus
import org.bitcoinppl.cove_core.CloudOnlyOperation
import org.bitcoinppl.cove_core.device.CloudSyncHealth
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

class CloudBackupUnplacedOutcomeTest {
    // a row delete that deleted nothing says so at the row's Delete; while a fetch reads the rows
    // again the screen lists no Not on This Device row, so the message moves under the bar instead
    // of disappearing until the fetch answers
    @Test
    fun aRowOutcomeShowsUnderTheBarWhileItsRowIsNotListed() {
        val outcome =
            CloudBackupLastOutcome(
                target = CloudBackupOutcomeTarget.Row(CloudBackupRowTarget(RECORD_ID, CloudBackupRowControl.DELETE)),
                outcome =
                    CloudBackupOutcome.Failed(
                        "This backup changed since you checked, so nothing was deleted. Check again.",
                    ),
            )

        val reading = configuredScreen(listsTheRow = false, lastOutcome = outcome)
        assertEquals(outcome, reading.unplacedOutcome())

        // once the rows are listed again, the row draws it next to its Delete
        assertNull(configuredScreen(listsTheRow = true, lastOutcome = outcome).unplacedOutcome())
    }

    // an action's outcome shows next to its control while Rust names one, and under the bar when
    // it names none; Switch Google Account's menu item can't hold a note, so its outcome always
    // shows under the bar
    @Test
    fun anActionOutcomeShowsUnderTheBarOnlyWithoutAControlForIt() {
        fun unplaced(action: CloudBackupScreenAction): CloudBackupOutcomeTarget? {
            val outcome =
                CloudBackupLastOutcome(CloudBackupOutcomeTarget.Action(action), CloudBackupOutcome.Failed("failed"))
            return configuredScreen(listsTheRow = true, lastOutcome = outcome).unplacedOutcome()?.target
        }

        assertNull(unplaced(CloudBackupScreenAction.VERIFY))
        assertEquals(
            CloudBackupOutcomeTarget.Action(CloudBackupScreenAction.SWITCH_DRIVE_ACCOUNT),
            unplaced(CloudBackupScreenAction.SWITCH_DRIVE_ACCOUNT),
        )
        assertEquals(
            CloudBackupOutcomeTarget.Action(CloudBackupScreenAction.REPAIR_PASSKEY),
            unplaced(CloudBackupScreenAction.REPAIR_PASSKEY),
        )
    }

    private companion object {
        const val RECORD_ID = "wallet-not-on-this-device"

        // a configured screen that offers Verify and Switch Google Account; without the row, it is
        // what Rust publishes while the cloud-only fetch reads the rows again
        fun configuredScreen(
            listsTheRow: Boolean,
            lastOutcome: CloudBackupLastOutcome,
        ): CloudBackupScreen {
            val notOnDevice =
                CloudBackupSection.NotOnDevice(
                    CloudBackupNotOnDeviceSection(
                        rows =
                            listOf(
                                CloudBackupListedRow(
                                    recordId = RECORD_ID,
                                    record = CloudBackupListedRecord.NotOnDevice(wallet()),
                                    actions = emptyList(),
                                ),
                            ),
                        restoreAll = CloudBackupRestoreAllState.NotShown,
                        operation = CloudOnlyOperation.Idle,
                    ),
                )
            val sections = listOfNotNull(notOnDevice.takeIf { listsTheRow }, CloudBackupSection.TurnOff)
            val offered = listOf(CloudBackupScreenAction.VERIFY, CloudBackupScreenAction.SWITCH_DRIVE_ACCOUNT)

            return CloudBackupScreen(
                body =
                    CloudBackupScreenBody.Configured(
                        CloudBackupConfiguredScreen(
                            sections = sections,
                            menu = listOf(CloudBackupMenuControl.SWITCH_DRIVE_ACCOUNT),
                            lastSync = null,
                            syncHealth = CloudSyncHealth.AllUploaded,
                            disable = CloudBackupDisableScreen.Idle,
                        ),
                    ),
                settingsRow = CloudBackupSettingsRowStatus.Syncing,
                healNotice = null,
                lastOutcome = lastOutcome,
                rootPrompt = CloudBackupRootPrompt.None,
                actions =
                    CloudBackupActions(
                        offered.map { CloudBackupActionEntry(it, CloudBackupActionAvailability.Available) },
                    ),
            )
        }

        fun wallet(): CloudBackupWalletItem =
            CloudBackupWalletItem(
                name = "Wallet",
                network = null,
                walletMode = null,
                walletType = null,
                fingerprint = null,
                labelCount = null,
                backupUpdatedAt = null,
                syncStatus = CloudBackupWalletStatus.DELETED_FROM_DEVICE,
                restoreFailure = null,
                recordId = RECORD_ID,
            )
    }
}
