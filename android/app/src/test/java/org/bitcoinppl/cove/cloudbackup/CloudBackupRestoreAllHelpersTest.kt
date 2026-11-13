package org.bitcoinppl.cove.cloudbackup

import org.bitcoinppl.cove_core.CloudBackupManagerAction
import org.bitcoinppl.cove_core.CloudBackupRestoreAllState
import org.bitcoinppl.cove_core.CloudBackupScreenAction
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class CloudBackupRestoreAllHelpersTest {
    @Test
    fun startStatesDrawRestoreAllWhoseAvailabilityComesFromActions() {
        val available =
            requireNotNull(
                cloudBackupRestoreAllAction(
                    CloudBackupRestoreAllState.StartAvailable(walletCount = 3u),
                ),
            )
        val disabled =
            requireNotNull(
                cloudBackupRestoreAllAction(
                    CloudBackupRestoreAllState.StartDisabled(walletCount = 3u),
                ),
            )

        assertEquals(CloudBackupScreenAction.START_RESTORE_ALL, available.action)
        assertEquals("Restore All (3)", available.title)
        assertEquals(available, disabled)
        assertEquals(CloudBackupManagerAction.StartRestoreAll, available.managerAction)
    }

    @Test
    fun retryAvailabilityNeverRetainsAStartAction() {
        val available =
            requireNotNull(
                cloudBackupRestoreAllAction(
                    CloudBackupRestoreAllState.RetryAvailable(walletCount = 1u),
                ),
            )
        val disabled =
            requireNotNull(
                cloudBackupRestoreAllAction(
                    CloudBackupRestoreAllState.RetryDisabled(walletCount = 1u),
                ),
            )

        assertEquals(CloudBackupScreenAction.RETRY_RESTORE_ALL_REMAINING, available.action)
        assertEquals("Retry Remaining (1)", available.title)
        assertEquals(available, disabled)
        assertEquals(CloudBackupManagerAction.RetryRestoreAllRemaining, available.managerAction)
    }

    @Test
    fun notShownAndRunningStatesExposeNoStaleSectionAction() {
        assertNull(cloudBackupRestoreAllAction(CloudBackupRestoreAllState.NotShown))
        assertNull(
            cloudBackupRestoreAllAction(
                CloudBackupRestoreAllState.Running(
                    completed = 1u,
                    total = 2u,
                    currentWalletName = "Savings",
                    cancellationRequested = false,
                ),
            ),
        )
    }

    @Test
    fun runningStateProjectsBoundedDeterminateProgressAndCurrentWallet() {
        val progress =
            requireNotNull(
                cloudBackupRestoreAllProgress(
                    CloudBackupRestoreAllState.Running(
                        completed = 2u,
                        total = 5u,
                        currentWalletName = "Savings",
                        cancellationRequested = false,
                    ),
                ),
            )

        assertEquals(0.4f, progress.fraction)
        assertEquals("2 of 5 complete", progress.status)
        assertEquals("Restoring Savings", progress.detail)
        assertEquals(
            "2 of 5 complete. Restoring Savings",
            progress.accessibilityState,
        )

        val bounded =
            requireNotNull(
                cloudBackupRestoreAllProgress(
                    CloudBackupRestoreAllState.Running(
                        completed = 7u,
                        total = 5u,
                        currentWalletName = null,
                        cancellationRequested = false,
                    ),
                ),
            )
        assertEquals(1f, bounded.fraction)
    }

    @Test
    fun cancellationRequestKeepsProgressAndRemovesRepeatedCancelIntent() {
        val progress =
            requireNotNull(
                cloudBackupRestoreAllProgress(
                    CloudBackupRestoreAllState.Running(
                        completed = 2u,
                        total = 5u,
                        currentWalletName = "Savings",
                        cancellationRequested = true,
                    ),
                ),
            )

        assertTrue(progress.cancellationRequested)
        assertEquals("Finishing the current wallet before stopping", progress.detail)
        assertEquals(0.4f, progress.fraction)
    }
}
