package org.bitcoinppl.cove.cloudbackup

import android.content.res.Configuration
import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.ui.tooling.preview.Preview
import org.bitcoinppl.cove.ui.theme.CoveTheme
import org.bitcoinppl.cove_core.CloudBackupConfiguredState
import org.bitcoinppl.cove_core.CloudBackupDestructiveOperationState
import org.bitcoinppl.cove_core.CloudBackupDetail
import org.bitcoinppl.cove_core.CloudBackupDetailState
import org.bitcoinppl.cove_core.CloudBackupLifecycle
import org.bitcoinppl.cove_core.CloudBackupOtherBackupsState
import org.bitcoinppl.cove_core.CloudBackupOtherBackupsSummary
import org.bitcoinppl.cove_core.CloudBackupPasskeyState
import org.bitcoinppl.cove_core.CloudBackupRootPrompt
import org.bitcoinppl.cove_core.CloudBackupRestoreAllState
import org.bitcoinppl.cove_core.CloudBackupState
import org.bitcoinppl.cove_core.CloudBackupSettingsRowStatus
import org.bitcoinppl.cove_core.CloudBackupSyncState
import org.bitcoinppl.cove_core.CloudBackupVerificationPresentation
import org.bitcoinppl.cove_core.CloudBackupVerificationState
import org.bitcoinppl.cove_core.CloudBackupWalletItem
import org.bitcoinppl.cove_core.CloudBackupWalletStatus
import org.bitcoinppl.cove_core.CloudOnlyOperation
import org.bitcoinppl.cove_core.CloudOnlyState
import org.bitcoinppl.cove_core.DeepVerificationReport
import org.bitcoinppl.cove_core.LoadedCloudBackupDetail
import org.bitcoinppl.cove_core.OtherBackupsOperation
import org.bitcoinppl.cove_core.WalletMode
import org.bitcoinppl.cove_core.WalletType
import org.bitcoinppl.cove_core.device.CloudSyncHealth
import org.bitcoinppl.cove_core.types.Network

@Preview(
    name = "Cloud Backup Detail Dark",
    widthDp = 393,
    heightDp = 852,
    showSystemUi = true,
    uiMode = Configuration.UI_MODE_NIGHT_YES,
)
@Composable
private fun CloudBackupScreenPreview() {
    CloudBackupScreenPreviewContent()
}

@Preview(
    name = "Cloud Backup Detail Light",
    widthDp = 393,
    heightDp = 852,
    showSystemUi = true,
    uiMode = Configuration.UI_MODE_NIGHT_NO,
)
@Composable
private fun CloudBackupScreenLightPreview() {
    CloudBackupScreenPreviewContent()
}

@Composable
internal fun CloudBackupScreenPreviewContent(darkTheme: Boolean = isSystemInDarkTheme()) {
    val manager = remember { CloudBackupManager(cloudBackupPreviewState()) }

    CoveTheme(darkTheme = darkTheme, dynamicColor = false) {
        CloudBackupScreenFrame(
            manager = manager,
            actions =
                CloudBackupScreenActions(
                    onBack = {},
                    onRecreate = {},
                    onReinitialize = {},
                ),
        )
    }
}

private fun cloudBackupPreviewState(): CloudBackupState {
    val detail =
        CloudBackupDetail(
            lastSync = 1_779_915_780UL,
            upToDate =
                listOf(
                    cloudBackupPreviewWallet(
                        name = "Wallet 1",
                        fingerprint = "55C5625F",
                        status = CloudBackupWalletStatus.CONFIRMED,
                        updatedAt = 1_779_915_780UL,
                    ),
                    cloudBackupPreviewWallet(
                        name = "Wallet 2",
                        fingerprint = "00053556",
                        status = CloudBackupWalletStatus.CONFIRMED,
                        updatedAt = 1_779_930_960UL,
                    ),
                    cloudBackupPreviewWallet(
                        name = "Imported 73C5DA0A",
                        fingerprint = "73C5DA0A",
                        walletType = WalletType.COLD,
                        status = CloudBackupWalletStatus.CONFIRMED,
                        updatedAt = 1_779_931_080UL,
                    ),
                ),
            needsSync = emptyList(),
            cloudOnlyCount = 1u,
            otherBackups =
                CloudBackupOtherBackupsState.Loaded(
                    CloudBackupOtherBackupsSummary(
                        namespaceCount = 0u,
                        walletCount = 0u,
                        passkeyHints = emptyList(),
                    ),
                ),
        )
    val loadedDetail =
        LoadedCloudBackupDetail(
            detail = detail,
            cloudOnly =
                CloudOnlyState.Loaded(
                    listOf(
                        cloudBackupPreviewWallet(
                            name = "Wallet 3",
                            fingerprint = "73C5DA0A",
                            status = CloudBackupWalletStatus.DELETED_FROM_DEVICE,
                            updatedAt = 1_779_931_020UL,
                        ),
                    ),
                ),
            cloudOnlyOperation = CloudOnlyOperation.Idle,
            otherBackupsOperation = OtherBackupsOperation.Idle,
        )

    return CloudBackupState(
        lifecycle =
            CloudBackupLifecycle.Configured(
                CloudBackupConfiguredState(
                    passkey = CloudBackupPasskeyState.Available,
                    verification =
                        CloudBackupVerificationState.Verified(
                            report =
                                DeepVerificationReport(
                                    masterKeyWrapperRepaired = true,
                                    localMasterKeyRepaired = false,
                                    credentialRecovered = false,
                                    walletsVerified = 4u,
                                    walletsFailed = 0u,
                                    walletsUnsupported = 0u,
                                    detail = detail,
                                ),
                            lastVerifiedAt = 1_779_930_000UL,
                        ),
                    sync = CloudBackupSyncState.Syncing,
                    destructiveOperation = CloudBackupDestructiveOperationState.Idle,
                    detail = CloudBackupDetailState.Complete(loadedDetail),
                    restoreAll = CloudBackupRestoreAllState.StartAvailable(walletCount = 2u),
                    rootPrompt = CloudBackupRootPrompt.None,
                    syncHealth = CloudSyncHealth.Uploading,
                    verificationPresentation = CloudBackupVerificationPresentation.Hidden(null),
                ),
            ),
        settingsRowStatus = CloudBackupSettingsRowStatus.Syncing,
    )
}

private fun cloudBackupPreviewWallet(
    name: String,
    fingerprint: String,
    status: CloudBackupWalletStatus,
    updatedAt: ULong,
    walletType: WalletType = WalletType.HOT,
): CloudBackupWalletItem =
    CloudBackupWalletItem(
        name = name,
        network = Network.BITCOIN,
        walletMode = WalletMode.MAIN,
        walletType = walletType,
        fingerprint = fingerprint,
        labelCount = 0u,
        backupUpdatedAt = updatedAt,
        syncStatus = status,
        restoreFailure = null,
        recordId = name,
    )
