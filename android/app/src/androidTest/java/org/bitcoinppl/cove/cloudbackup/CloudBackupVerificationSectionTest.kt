package org.bitcoinppl.cove.cloudbackup

import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.getUnclippedBoundsInRoot
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import org.bitcoinppl.cove.ui.theme.CoveTheme
import org.bitcoinppl.cove_core.CloudBackupConfiguredState
import org.bitcoinppl.cove_core.CloudBackupDestructiveOperationState
import org.bitcoinppl.cove_core.CloudBackupDetailState
import org.bitcoinppl.cove_core.CloudBackupLifecycle
import org.bitcoinppl.cove_core.CloudBackupOtherBackupsState
import org.bitcoinppl.cove_core.CloudBackupPasskeyState
import org.bitcoinppl.cove_core.CloudBackupRestoreAllState
import org.bitcoinppl.cove_core.CloudBackupRootPrompt
import org.bitcoinppl.cove_core.CloudBackupSettingsRowStatus
import org.bitcoinppl.cove_core.CloudBackupState
import org.bitcoinppl.cove_core.CloudBackupSyncState
import org.bitcoinppl.cove_core.CloudBackupUndecryptableWalletDeletionState
import org.bitcoinppl.cove_core.CloudBackupVerificationPresentation
import org.bitcoinppl.cove_core.CloudBackupVerificationState
import org.bitcoinppl.cove_core.CloudBackupWalletVerificationIssues
import org.bitcoinppl.cove_core.DeepVerificationFailure
import org.bitcoinppl.cove_core.DeepVerificationReport
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

    @Test
    fun undecryptableIssueRequiresDeletionConfirmation() {
        val issue = "3 wallet backup(s) could not be decrypted with this backup key"
        val manager =
            configuredManager(
                CloudBackupVerificationState.NeedsAttention(
                    report =
                        DeepVerificationReport(
                            masterKeyWrapperRepaired = false,
                            localMasterKeyRepaired = false,
                            credentialRecovered = false,
                            walletsVerified = 2u,
                            walletIssues =
                                CloudBackupWalletVerificationIssues(
                                    missing = 0u,
                                    downloadFailed = 0u,
                                    invalid = 0u,
                                    decryptionFailed = 3u,
                                    unsupported = 0u,
                                    unreadable = 0u,
                                ),
                            detail = null,
                        ),
                    checkedAt = 1UL,
                ),
            )

        compose.setContent {
            CoveTheme(darkTheme = false) {
                VerificationSection(
                    manager = manager,
                    onRecreate = {},
                    onReinitialize = {},
                )
            }
        }

        compose.onNodeWithText(issue).performClick()

        compose.onNodeWithText("Delete 3 Inaccessible Wallet Backups?").assertIsDisplayed()
        compose.onNodeWithText("Delete Backups").assertIsDisplayed()
        compose.onNodeWithText("Cancel").assertIsDisplayed()
    }

    private fun failedVerificationManager(error: String): CloudBackupManager =
        configuredManager(
            CloudBackupVerificationState.Failed(
                DeepVerificationFailure.Retry(
                    message = error,
                    detail = null,
                    retryAction = null,
                ),
            ),
        )

    private fun configuredManager(verification: CloudBackupVerificationState): CloudBackupManager =
        CloudBackupManager(
            CloudBackupState(
                lifecycle =
                    CloudBackupLifecycle.Configured(
                        CloudBackupConfiguredState(
                            passkey = CloudBackupPasskeyState.Available,
                            verification = verification,
                            sync = CloudBackupSyncState.Idle,
                            destructiveOperation = CloudBackupDestructiveOperationState.Idle,
                            undecryptableWalletDeletion =
                                CloudBackupUndecryptableWalletDeletionState.Idle,
                            detail = CloudBackupDetailState.NotLoaded,
                            otherBackups = CloudBackupOtherBackupsState.NotChecked,
                            restoreAll = CloudBackupRestoreAllState.NotShown,
                            rootPrompt = CloudBackupRootPrompt.None,
                            syncHealth = CloudSyncHealth.Unknown,
                            verificationPresentation = CloudBackupVerificationPresentation.Hidden(null),
                        ),
                    ),
                settingsRowStatus = CloudBackupSettingsRowStatus.CheckingSync,
            ),
        )
}
