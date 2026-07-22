package org.bitcoinppl.cove.cloudbackup

import org.bitcoinppl.cove_core.CloudBackupManagerAction
import org.bitcoinppl.cove_core.CloudBackupRestoreAllState
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class CloudBackupRestoreAllHelpersTest {
    @Test
    fun startAvailabilityProjectsOnlyTheRustOwnedCountAndEnabledState() {
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

        assertEquals(CloudBackupRestoreAllActionKind.START, available.kind)
        assertEquals("Restore All (3)", available.title)
        assertTrue(available.enabled)
        assertEquals(available.title, disabled.title)
        assertFalse(disabled.enabled)
        assertEquals(
            CloudBackupManagerAction.StartRestoreAll,
            cloudBackupRestoreAllManagerAction(available.kind),
        )
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

        assertEquals(CloudBackupRestoreAllActionKind.RETRY, available.kind)
        assertEquals("Retry Remaining (1)", available.title)
        assertTrue(available.enabled)
        assertEquals(available.title, disabled.title)
        assertFalse(disabled.enabled)
        assertEquals(
            CloudBackupManagerAction.RetryRestoreAllRemaining,
            cloudBackupRestoreAllManagerAction(available.kind),
        )
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
