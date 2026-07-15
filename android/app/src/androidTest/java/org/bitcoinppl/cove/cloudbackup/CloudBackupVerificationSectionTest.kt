package org.bitcoinppl.cove.cloudbackup

import androidx.compose.ui.test.getUnclippedBoundsInRoot
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithText
import org.bitcoinppl.cove.ui.theme.CoveTheme
import org.bitcoinppl.cove_core.CloudBackupConfiguredState
import org.bitcoinppl.cove_core.CloudBackupDestructiveOperationState
import org.bitcoinppl.cove_core.CloudBackupDetailState
import org.bitcoinppl.cove_core.CloudBackupLifecycle
import org.bitcoinppl.cove_core.CloudBackupPasskeyState
import org.bitcoinppl.cove_core.CloudBackupRootPrompt
import org.bitcoinppl.cove_core.CloudBackupSettingsRowStatus
import org.bitcoinppl.cove_core.CloudBackupState
import org.bitcoinppl.cove_core.CloudBackupSyncState
import org.bitcoinppl.cove_core.CloudBackupVerificationPresentation
import org.bitcoinppl.cove_core.CloudBackupVerificationState
import org.bitcoinppl.cove_core.DeepVerificationFailure
import org.bitcoinppl.cove_core.device.CloudSyncHealth
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test

class CloudBackupVerificationSectionTest {
    @get:Rule
    val compose = createComposeRule()

    @Test
    fun retryActionsAreLaidOutBelowErrorMessage() {
        val error =
            "cloud storage error: failed to download master key backup: " +
                "authorization required: google drive authorization was cancelled"
        val manager = failedVerificationManager(error)

        compose.setContent {
            CoveTheme(darkTheme = false) {
                VerificationSection(
                    manager = manager,
                    onRecreate = {},
                    onReinitialize = {},
                )
            }
        }

        val errorBottom = compose.onNodeWithText(error).getUnclippedBoundsInRoot().bottom
        val retryTop = compose.onNodeWithText("Try Again").getUnclippedBoundsInRoot().top
        val createPasskeyTop = compose.onNodeWithText("Create New Passkey").getUnclippedBoundsInRoot().top

        assertTrue("retry action should appear below the error", retryTop >= errorBottom)
        assertTrue("create-passkey action should appear below retry", createPasskeyTop > retryTop)
    }

    private fun failedVerificationManager(error: String): CloudBackupManager =
        CloudBackupManager(
            CloudBackupState(
                lifecycle =
                    CloudBackupLifecycle.Configured(
                        CloudBackupConfiguredState(
                            passkey = CloudBackupPasskeyState.Available,
                            verification =
                                CloudBackupVerificationState.Failed(
                                    DeepVerificationFailure.Retry(
                                        message = error,
                                        detail = null,
                                        retryContext = null,
                                    ),
                                ),
                            sync = CloudBackupSyncState.Idle,
                            destructiveOperation = CloudBackupDestructiveOperationState.Idle,
                            detail = CloudBackupDetailState.NotLoaded,
                            rootPrompt = CloudBackupRootPrompt.None,
                            syncHealth = CloudSyncHealth.Unknown,
                            verificationPresentation = CloudBackupVerificationPresentation.Hidden(null),
                        ),
                    ),
                settingsRowStatus = CloudBackupSettingsRowStatus.CheckingSync,
            ),
        )
}
