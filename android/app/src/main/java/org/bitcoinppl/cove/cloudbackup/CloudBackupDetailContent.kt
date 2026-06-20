package org.bitcoinppl.cove.cloudbackup

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.CloudOff
import androidx.compose.material.icons.filled.CloudUpload
import androidx.compose.material.icons.filled.Key
import androidx.compose.material.icons.filled.WarningAmber
import androidx.compose.material3.Button
import androidx.compose.material3.Card
import androidx.compose.material3.CardDefaults
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import org.bitcoinppl.cove.R
import org.bitcoinppl.cove_core.CloudBackupDetail
import org.bitcoinppl.cove_core.CloudBackupDetailState
import org.bitcoinppl.cove_core.CloudBackupManagerAction
import org.bitcoinppl.cove_core.CloudBackupOtherBackupsState
import org.bitcoinppl.cove_core.CloudBackupPasskeyRepairState
import org.bitcoinppl.cove_core.CloudBackupSyncState
import org.bitcoinppl.cove_core.CloudBackupVerificationSource
import org.bitcoinppl.cove_core.CloudBackupVerificationState
import org.bitcoinppl.cove_core.CloudOnlyState
import org.bitcoinppl.cove_core.device.CloudSyncHealth

internal enum class CloudBackupDetailBodyState {
    UNSUPPORTED_PASSKEY_PROVIDER,
    MISSING_PASSKEY,
    VERIFYING,
    DETAIL,
    AUTHORIZATION_BLOCKED,
    LOADING,
}

internal fun cloudBackupDetailBodyState(
    manager: CloudBackupManager,
    hasDetail: Boolean,
): CloudBackupDetailBodyState? =
    when {
        manager.isUnsupportedPasskeyProvider -> CloudBackupDetailBodyState.UNSUPPORTED_PASSKEY_PROVIDER
        manager.isPasskeyMissing -> CloudBackupDetailBodyState.MISSING_PASSKEY
        manager.verificationState is CloudBackupVerificationState.Running -> CloudBackupDetailBodyState.VERIFYING
        hasDetail -> CloudBackupDetailBodyState.DETAIL
        manager.hasPendingUploadVerification && manager.syncState is CloudBackupSyncState.Blocked ->
            CloudBackupDetailBodyState.AUTHORIZATION_BLOCKED
        manager.verificationState !is CloudBackupVerificationState.Failed -> CloudBackupDetailBodyState.LOADING
        else -> null
    }

internal fun shouldShowPendingUploadConfirmationStatus(
    manager: CloudBackupManager,
): Boolean = manager.hasPendingUploadVerification

internal fun shouldFetchCloudOnly(cloudOnly: CloudOnlyState): Boolean =
    cloudOnly is CloudOnlyState.NotFetched

internal fun shouldShowFallbackVerificationSection(
    bodyState: CloudBackupDetailBodyState?,
): Boolean = bodyState == null

@Composable
internal fun CloudBackupDetailContent(
    manager: CloudBackupManager,
    headerError: String?,
    onRecreate: () -> Unit,
    onReinitialize: () -> Unit,
) {
    val bodyState =
        cloudBackupDetailBodyState(
            manager = manager,
            hasDetail = manager.detail != null,
        )
    val showFallbackVerificationSection = shouldShowFallbackVerificationSection(bodyState)

    Column(
        modifier =
            Modifier
                .fillMaxSize()
                .verticalScroll(rememberScrollState())
                .padding(bottom = 24.dp),
    ) {
        headerError?.let {
            ErrorInlineMessage(it, modifier = Modifier.padding(horizontal = 16.dp, vertical = 8.dp))
        }

        when (bodyState) {
            CloudBackupDetailBodyState.UNSUPPORTED_PASSKEY_PROVIDER -> {
                UnsupportedPasskeyProviderContent(manager = manager)
            }
            CloudBackupDetailBodyState.MISSING_PASSKEY -> {
                MissingPasskeyContent(manager = manager)
            }
            CloudBackupDetailBodyState.VERIFYING -> {
                CloudBackupProgressCard(
                    title = stringResource(R.string.cloud_backup_detail_verifying_title),
                    message = stringResource(R.string.cloud_backup_detail_verifying_message),
                )
            }
            CloudBackupDetailBodyState.DETAIL -> {
                DetailFormContent(
                    detail = manager.detail!!,
                    syncHealth = manager.syncHealth,
                    manager = manager,
                )
            }
            CloudBackupDetailBodyState.AUTHORIZATION_BLOCKED -> {
                PendingUploadConfirmationStatus(isBlockedOnAuthorization = true)
            }
            CloudBackupDetailBodyState.LOADING -> {
                CloudBackupProgressCard(
                    title = stringResource(R.string.cloud_backup_detail_loading_title),
                    message = stringResource(R.string.cloud_backup_detail_loading_message),
                )
            }
            null -> Unit
        }

        if (
            bodyState != CloudBackupDetailBodyState.AUTHORIZATION_BLOCKED &&
                shouldShowPendingUploadConfirmationStatus(manager)
        ) {
            PendingUploadConfirmationStatus(isBlockedOnAuthorization = manager.syncState is CloudBackupSyncState.Blocked)
        }

        if (showFallbackVerificationSection) {
            VerificationSection(
                manager = manager,
                onRecreate = onRecreate,
                onReinitialize = onReinitialize,
            )
        }

        if (bodyState == CloudBackupDetailBodyState.DETAIL) {
            VerificationSection(
                manager = manager,
                onRecreate = onRecreate,
                onReinitialize = onReinitialize,
            )
        }

        if (
            bodyState == CloudBackupDetailBodyState.DETAIL ||
                bodyState == CloudBackupDetailBodyState.MISSING_PASSKEY
        ) {
            DisableCloudBackupSection(manager = manager, detail = manager.detail)
        }
    }
}

@Composable
private fun PendingUploadConfirmationStatus(
    isBlockedOnAuthorization: Boolean,
) {
    if (isBlockedOnAuthorization) {
        Card(
            modifier = Modifier.padding(horizontal = 16.dp, vertical = 8.dp),
            colors =
                CardDefaults.cardColors(
                    containerColor = MaterialTheme.colorScheme.errorContainer,
                ),
        ) {
            Column(modifier = Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                Text(
                    stringResource(R.string.cloud_backup_authorization_needed_title),
                    style = MaterialTheme.typography.titleMedium,
                    color = MaterialTheme.colorScheme.onErrorContainer,
                )
                Text(
                    stringResource(R.string.cloud_backup_authorization_needed_message),
                    style = MaterialTheme.typography.bodyMedium,
                    color = MaterialTheme.colorScheme.onErrorContainer,
                )
            }
        }
    } else {
        CloudBackupProgressCard(
            title = stringResource(R.string.cloud_backup_confirming_upload_title),
            message = stringResource(R.string.cloud_backup_confirming_upload_message),
        )
    }
}

@Composable
private fun UnsupportedPasskeyProviderContent(
    manager: CloudBackupManager,
) {
    Column(modifier = Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(16.dp)) {
        ErrorStateCard(
            icon = Icons.Default.WarningAmber,
            title = stringResource(R.string.cloud_backup_unsupported_passkey_title),
            body = stringResource(R.string.cloud_backup_unsupported_passkey_body),
        )

        Button(
            onClick = {
                manager.dispatch(
                    manualEnableCloudBackupNoDiscovery(
                        CloudBackupVerificationSource.CLOUD_BACKUP_DETAIL,
                    ),
                )
            },
            modifier = Modifier.fillMaxWidth(),
        ) {
            Text(stringResource(R.string.action_try_again))
        }
    }
}

@Composable
private fun MissingPasskeyContent(
    manager: CloudBackupManager,
) {
    val repairState = manager.passkeyRepairState
    val isRepairing = repairState is CloudBackupPasskeyRepairState.Running
    val repairError =
        if (repairState is CloudBackupPasskeyRepairState.Failed) {
            repairState.v1
        } else {
            null
        }

    Column(modifier = Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(16.dp)) {
        ErrorStateCard(
            icon = Icons.Default.Key,
            title = stringResource(R.string.cloud_backup_missing_passkey_title),
            body = stringResource(R.string.cloud_backup_missing_passkey_body),
        )

        Button(
            onClick = { manager.dispatch(CloudBackupManagerAction.RepairPasskeyNoDiscovery) },
            enabled = !isRepairing,
            modifier = Modifier.fillMaxWidth(),
        ) {
            if (isRepairing) {
                CircularProgressIndicator(modifier = Modifier.width(18.dp).height(18.dp), strokeWidth = 2.dp)
                Spacer(modifier = Modifier.width(8.dp))
            }
            Text(
                if (isRepairing) {
                    stringResource(R.string.settings_action_opening_passkey_options)
                } else {
                    stringResource(R.string.settings_action_add_passkey)
                },
            )
        }

        repairError?.let {
            ErrorInlineMessage(it)
        }
    }
}

@Composable
private fun DetailFormContent(
    detail: CloudBackupDetail,
    syncHealth: CloudSyncHealth,
    manager: CloudBackupManager,
) {
    val showCloudOnlySection =
        when (val cloudOnly = manager.cloudOnly) {
            is CloudOnlyState.NotFetched -> detail.cloudOnlyCount.toInt() > 0
            is CloudOnlyState.Loading -> true
            is CloudOnlyState.Loaded -> cloudOnly.wallets.isNotEmpty()
            is CloudOnlyState.Failed -> true
        }

    Column(verticalArrangement = Arrangement.spacedBy(CloudBackupDetailSectionSpacing)) {
        CloudBackupHeaderSection(lastSync = detail.lastSync, syncHealth = syncHealth)

        if (detail.upToDate.isNotEmpty()) {
            WalletSections(title = stringResource(R.string.cloud_backup_wallet_section_up_to_date), wallets = detail.upToDate)
        }

        if (detail.needsSync.isNotEmpty()) {
            WalletSections(title = stringResource(R.string.cloud_backup_wallet_section_needs_sync), wallets = detail.needsSync)
        }

        if (showCloudOnlySection) {
            CloudOnlySection(manager = manager)
        }

        when (val otherBackups = detail.otherBackups) {
            is CloudBackupOtherBackupsState.Loaded -> {
                val summary = otherBackups.summary
                if (summary.namespaceCount.toInt() > 0) {
                    OtherBackupsSection(
                        namespaceCount = summary.namespaceCount.toInt(),
                        walletCount = summary.walletCount.toInt(),
                        passkeySuffixes = summary.passkeyHints.map { it.nameSuffix },
                        manager = manager,
                    )
                }
            }
            is CloudBackupOtherBackupsState.LoadFailed -> {
                OtherBackupsLoadFailedSection(error = otherBackups.error)
            }
        }
    }
}
