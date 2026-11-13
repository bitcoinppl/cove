package org.bitcoinppl.cove.cloudbackup

import android.content.res.Configuration
import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.ui.tooling.preview.Preview
import org.bitcoinppl.cove.ui.theme.CoveTheme
import org.bitcoinppl.cove_core.CloudBackupActionAvailability
import org.bitcoinppl.cove_core.CloudBackupActionEntry
import org.bitcoinppl.cove_core.CloudBackupActions
import org.bitcoinppl.cove_core.CloudBackupConfiguredScreen
import org.bitcoinppl.cove_core.CloudBackupDeleteCategory
import org.bitcoinppl.cove_core.CloudBackupDeleteConfirmation
import org.bitcoinppl.cove_core.CloudBackupDeleteEvidence
import org.bitcoinppl.cove_core.CloudBackupDetail
import org.bitcoinppl.cove_core.CloudBackupDisableScreen
import org.bitcoinppl.cove_core.CloudBackupListedRecord
import org.bitcoinppl.cove_core.CloudBackupListedRow
import org.bitcoinppl.cove_core.CloudBackupMenuControl
import org.bitcoinppl.cove_core.CloudBackupNotOnDeviceSection
import org.bitcoinppl.cove_core.CloudBackupOwnRecord
import org.bitcoinppl.cove_core.CloudBackupOwnRow
import org.bitcoinppl.cove_core.CloudBackupRecoveryOperation
import org.bitcoinppl.cove_core.CloudBackupRestoreAllState
import org.bitcoinppl.cove_core.CloudBackupRootPrompt
import org.bitcoinppl.cove_core.CloudBackupRowAction
import org.bitcoinppl.cove_core.CloudBackupRowActionKind
import org.bitcoinppl.cove_core.CloudBackupRowDeleteConsent
import org.bitcoinppl.cove_core.CloudBackupScreen
import org.bitcoinppl.cove_core.CloudBackupScreenAction
import org.bitcoinppl.cove_core.CloudBackupScreenBody
import org.bitcoinppl.cove_core.CloudBackupSection
import org.bitcoinppl.cove_core.CloudBackupSettingsRowStatus
import org.bitcoinppl.cove_core.CloudBackupSyncState
import org.bitcoinppl.cove_core.CloudBackupVerificationControl
import org.bitcoinppl.cove_core.CloudBackupVerificationPresentation
import org.bitcoinppl.cove_core.CloudBackupVerificationSection
import org.bitcoinppl.cove_core.CloudBackupVerificationState
import org.bitcoinppl.cove_core.CloudBackupVerificationView
import org.bitcoinppl.cove_core.CloudBackupWalletItem
import org.bitcoinppl.cove_core.CloudBackupWalletStatus
import org.bitcoinppl.cove_core.CloudBackupWalletVerificationIssues
import org.bitcoinppl.cove_core.CloudBackupWalletsSection
import org.bitcoinppl.cove_core.CloudOnlyOperation
import org.bitcoinppl.cove_core.DeepVerificationReport
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
    val manager = remember { CloudBackupManager(cloudBackupPreviewScreen()) }

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
// built in Kotlin rather than from the exported fixtures, because previews run without the
// Rust library
private fun cloudBackupPreviewScreen(): CloudBackupScreen {
    val upToDate =
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
        )
    val notOnDevice =
        cloudBackupPreviewWallet(
            name = "Wallet 3",
            fingerprint = "73C5DA0A",
            status = CloudBackupWalletStatus.DELETED_FROM_DEVICE,
            updatedAt = 1_779_931_020UL,
        )
    val verification =
        CloudBackupVerificationView(
            state =
                CloudBackupVerificationState.Verified(
                    report =
                        DeepVerificationReport(
                            masterKeyWrapperRepaired = true,
                            localMasterKeyRepaired = false,
                            credentialRecovered = false,
                            walletsVerified = 4u,
                            walletIssues =
                                CloudBackupWalletVerificationIssues(
                                    missing = 0u,
                                    downloadFailed = 0u,
                                    invalid = 0u,
                                    decryptionFailed = 0u,
                                    unsupported = 0u,
                                    unreadable = 0u,
                                ),
                            detail = CloudBackupDetail(1_779_915_780UL, upToDate, emptyList()),
                        ),
                    lastVerifiedAt = 1_779_930_000UL,
                ),
            presentation = CloudBackupVerificationPresentation.Hidden(null),
            recovery = CloudBackupRecoveryOperation.IDLE,
        )
    val available = CloudBackupActionAvailability.Available

    return CloudBackupScreen(
        body =
            CloudBackupScreenBody.Configured(
                CloudBackupConfiguredScreen(
                    sections =
                        listOf(
                            CloudBackupSection.Verification(
                                CloudBackupVerificationSection(
                                    view = verification,
                                    controls = listOf(CloudBackupVerificationControl.Verify),
                                ),
                            ),
                            CloudBackupSection.Wallets(
                                CloudBackupWalletsSection(
                                    upToDate = upToDate.map { CloudBackupOwnRow(it, CloudBackupOwnRecord.CURRENT) },
                                    needsSync = emptyList(),
                                    sync = CloudBackupSyncState.Syncing,
                                    controls = emptyList(),
                                ),
                            ),
                            CloudBackupSection.NotOnDevice(
                                CloudBackupNotOnDeviceSection(
                                    rows =
                                        listOf(
                                            CloudBackupListedRow(
                                                recordId = notOnDevice.recordId,
                                                record = CloudBackupListedRecord.NotOnDevice(notOnDevice),
                                                actions =
                                                    listOf(
                                                        CloudBackupRowAction(
                                                            CloudBackupRowActionKind.Restore,
                                                            available,
                                                        ),
                                                        CloudBackupRowAction(
                                                            CloudBackupRowActionKind.Delete(
                                                                CloudBackupDeleteConfirmation.MAY_BE_ONLY_COPY,
                                                                previewDeleteConsent(notOnDevice.recordId),
                                                            ),
                                                            available,
                                                        ),
                                                    ),
                                            ),
                                        ),
                                    restoreAll = CloudBackupRestoreAllState.StartAvailable(walletCount = 1u),
                                    operation = CloudOnlyOperation.Idle,
                                ),
                            ),
                            CloudBackupSection.TurnOff,
                        ),
                    menu = listOf(CloudBackupMenuControl.SWITCH_DRIVE_ACCOUNT),
                    lastSync = 1_779_915_780UL,
                    syncHealth = CloudSyncHealth.Uploading,
                    disable = CloudBackupDisableScreen.Idle,
                    passkeyHint = null,
                ),
            ),
        settingsRow = CloudBackupSettingsRowStatus.Syncing,
        healNotice = null,
        lastOutcome = null,
        rootPrompt = CloudBackupRootPrompt.None,
        actions =
            CloudBackupActions(
                listOf(
                    CloudBackupScreenAction.VERIFY,
                    CloudBackupScreenAction.SWITCH_DRIVE_ACCOUNT,
                    CloudBackupScreenAction.START_RESTORE_ALL,
                    CloudBackupScreenAction.TURN_OFF,
                ).map { CloudBackupActionEntry(it, available) },
            ),
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

/**
 * A consent for previews, which have no Rust inventory to mint one; only Rust reads its fields,
 * and nothing a preview draws dispatches it
 */
internal fun previewDeleteConsent(recordId: String): CloudBackupRowDeleteConsent =
    CloudBackupRowDeleteConsent(
        recordId = recordId,
        evidence =
            CloudBackupDeleteEvidence(
                category = CloudBackupDeleteCategory.NotOnDevice,
                namespaceId = "preview-namespace",
                configuration = 0uL,
                inventoryScope = null,
                listingGeneration = 0uL,
                localWalletsGeneration = 0uL,
            ),
    )
