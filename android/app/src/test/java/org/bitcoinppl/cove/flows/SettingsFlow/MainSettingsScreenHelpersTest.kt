package org.bitcoinppl.cove.flows.SettingsFlow

import org.bitcoinppl.cove.cloudbackup.CloudBackupManager
import org.bitcoinppl.cove_core.CloudBackupConfiguredState
import org.bitcoinppl.cove_core.CloudBackupDestructiveOperationState
import org.bitcoinppl.cove_core.CloudBackupDetailState
import org.bitcoinppl.cove_core.CloudBackupLifecycle
import org.bitcoinppl.cove_core.CloudBackupPasskeyState
import org.bitcoinppl.cove_core.CloudBackupRootPrompt
import org.bitcoinppl.cove_core.CloudBackupState
import org.bitcoinppl.cove_core.CloudBackupSyncState
import org.bitcoinppl.cove_core.CloudBackupVerificationPresentation
import org.bitcoinppl.cove_core.CloudBackupVerificationState
import org.bitcoinppl.cove_core.device.CloudSyncHealth
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class MainSettingsScreenHelpersTest {
    @Test
    fun cloudBackupSettingsStayHiddenInDecoyMode() {
        assertTrue(shouldShowCloudBackupSettings(isInDecoyMode = false))
        assertFalse(shouldShowCloudBackupSettings(isInDecoyMode = true))
    }

    @Test
    fun cloudBackupSettingsDoNotShowActiveForUnhealthySyncHealth() {
        assertTrue(
            cloudBackupSettingsStatus(cloudBackupManager(CloudSyncHealth.Unknown))
                is CloudBackupSettingsStatus.CheckingSync,
        )
        assertTrue(
            cloudBackupSettingsStatus(cloudBackupManager(CloudSyncHealth.AuthorizationRequired("wrong account")))
                is CloudBackupSettingsStatus.AuthorizationRequired,
        )
        assertTrue(
            cloudBackupSettingsStatus(cloudBackupManager(CloudSyncHealth.Failed("drive unavailable")))
                is CloudBackupSettingsStatus.Error,
        )
        assertTrue(
            cloudBackupSettingsStatus(cloudBackupManager(CloudSyncHealth.AllUploaded))
                is CloudBackupSettingsStatus.Active,
        )
    }

    private fun cloudBackupManager(syncHealth: CloudSyncHealth): CloudBackupManager {
        val state =
            CloudBackupState(
                lifecycle =
                    CloudBackupLifecycle.Configured(
                        CloudBackupConfiguredState(
                            passkey = CloudBackupPasskeyState.Available,
                            verification = CloudBackupVerificationState.NotVerified,
                            sync = CloudBackupSyncState.Idle,
                            destructiveOperation = CloudBackupDestructiveOperationState.Idle,
                            detail = CloudBackupDetailState.NotLoaded,
                            rootPrompt = CloudBackupRootPrompt.None,
                            syncHealth = syncHealth,
                            verificationPresentation = CloudBackupVerificationPresentation.Hidden(null),
                        ),
                    ),
            )

        return CloudBackupManager(state)
    }
}
