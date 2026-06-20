package org.bitcoinppl.cove.cloudbackup

import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Key
import androidx.compose.material.icons.filled.Refresh
import androidx.compose.material.icons.filled.Security
import androidx.compose.material.icons.filled.WarningAmber
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.res.pluralStringResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import org.bitcoinppl.cove.R
import org.bitcoinppl.cove.localizedMessage
import org.bitcoinppl.cove.localizedWarning
import org.bitcoinppl.cove.ui.theme.CoveColor
import org.bitcoinppl.cove.ui.theme.caption
import org.bitcoinppl.cove.views.MaterialDivider
import org.bitcoinppl.cove.views.MaterialSection
import org.bitcoinppl.cove.views.MaterialSettingsItem
import org.bitcoinppl.cove.views.SectionHeader
import org.bitcoinppl.cove_core.CloudBackupManagerAction
import org.bitcoinppl.cove_core.CloudBackupPasskeyRepairState
import org.bitcoinppl.cove_core.CloudBackupRetryAction
import org.bitcoinppl.cove_core.CloudBackupSyncState
import org.bitcoinppl.cove_core.CloudBackupVerificationSource
import org.bitcoinppl.cove_core.CloudBackupVerificationState
import org.bitcoinppl.cove_core.DeepVerificationFailure
import org.bitcoinppl.cove_core.DeepVerificationReport

@Composable
private fun CancelledVerificationRecoveryContent(
    manager: CloudBackupManager,
) {
    Column(modifier = Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(16.dp)) {
        ErrorStateCard(
            icon = Icons.Default.WarningAmber,
            title = stringResource(R.string.cloud_backup_verification_cancelled_title),
            body = stringResource(R.string.cloud_backup_verification_cancelled_body),
        )

        SectionHeader(stringResource(R.string.cloud_backup_verification_section_title))
        MaterialSection {
            Column {
                CancelledVerificationActions(manager = manager)
            }
        }
    }
}

@Composable
private fun PasskeyConfirmedSectionContent(
    manager: CloudBackupManager,
) {
    CloudBackupSimpleActionCard(
        title = stringResource(R.string.cloud_backup_verification_passkey_verified),
        icon = Icons.Default.Security,
        tint = cloudBackupVisualColors().success,
        onClick = {
            manager.dispatch(
                CloudBackupManagerAction.StartVerification(
                    CloudBackupVerificationSource.CLOUD_BACKUP_DETAIL,
                ),
            )
        },
    )
}

@Composable
internal fun VerificationSection(
    manager: CloudBackupManager,
    onRecreate: () -> Unit,
    onReinitialize: () -> Unit,
) {
    val colors = cloudBackupVisualColors()

    Column(
        modifier = Modifier.padding(top = 10.dp),
        verticalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        when (val verification = manager.verificationState) {
            null,
            CloudBackupVerificationState.NotVerified,
            CloudBackupVerificationState.Required,
            -> {
                CloudBackupSimpleActionCard(
                    title = stringResource(R.string.cloud_backup_verification_verify_now),
                    icon = Icons.Default.Security,
                    tint = colors.cloudBlue,
                    onClick = {
                        manager.dispatch(
                            CloudBackupManagerAction.StartVerification(
                                CloudBackupVerificationSource.CLOUD_BACKUP_DETAIL,
                            ),
                        )
                    },
                )
            }

            CloudBackupVerificationState.Running -> {
                CloudBackupProgressCard(
                    title = stringResource(R.string.cloud_backup_verification_integrity_title),
                    message = stringResource(R.string.cloud_backup_verification_integrity_message),
                )
            }

            is CloudBackupVerificationState.Verified -> {
                val report = verification.report
                if (report != null) {
                    VerifiedSectionContent(report = report)
                } else {
                    PasskeyConfirmedSectionContent(manager)
                }
            }

            CloudBackupVerificationState.AwaitingUploadConfirmation -> {
                PasskeyConfirmedSectionContent(manager)
            }

            is CloudBackupVerificationState.Failed -> {
                CloudBackupGlassCard(
                    modifier =
                        Modifier
                            .fillMaxWidth()
                            .padding(horizontal = 14.dp),
                ) {
                    VerificationFailureContent(
                        failure = verification.v1,
                        manager = manager,
                        onRecreate = onRecreate,
                        onReinitialize = onReinitialize,
                    )
                }
            }
        }

        when (manager.syncState) {
            CloudBackupSyncState.FAILED -> {
                ErrorInlineMessage(stringResource(R.string.cloud_backup_sync_failed), modifier = Modifier.padding(horizontal = 14.dp))
            }

            CloudBackupSyncState.BLOCKED -> {
                ErrorInlineMessage(stringResource(R.string.cloud_backup_sync_blocked), modifier = Modifier.padding(horizontal = 14.dp))
            }

            else -> Unit
        }

        val needsSync = manager.detail?.needsSync?.isNotEmpty() == true
        if (needsSync) {
            CloudBackupSimpleActionCard(
                title = stringResource(R.string.cloud_backup_verification_sync_now),
                icon = Icons.Default.Refresh,
                tint = colors.cloudBlue,
                onClick = { manager.dispatch(CloudBackupManagerAction.SyncUnsynced) },
            )
        }

        if (manager.verificationState is CloudBackupVerificationState.Verified) {
            VerifyAgainButton(
                onClick = {
                    manager.dispatch(
                        CloudBackupManagerAction.StartVerification(
                            CloudBackupVerificationSource.CLOUD_BACKUP_DETAIL,
                        ),
                    )
                },
            )
        }
    }
}

@Composable
private fun CancelledVerificationActions(
    manager: CloudBackupManager,
) {
    MaterialSettingsItem(
        title = stringResource(R.string.cloud_backup_verification_cancelled_title),
        subtitle = stringResource(R.string.cloud_backup_verification_cancelled_subtitle),
        onClick = {
            manager.dispatch(
                CloudBackupManagerAction.StartVerification(
                    CloudBackupVerificationSource.CLOUD_BACKUP_DETAIL,
                ),
            )
        },
        leadingContent = { Icon(Icons.Default.WarningAmber, contentDescription = null, tint = CoveColor.WarningOrange) },
    )
    MaterialDivider()
    MaterialSettingsItem(
        title = stringResource(R.string.settings_action_create_new_passkey),
        onClick = { manager.dispatch(CloudBackupManagerAction.RepairPasskeyNoDiscovery) },
        leadingContent = { Icon(Icons.Default.Key, contentDescription = null) },
    )
}

@Composable
private fun VerifiedSectionContent(
    report: DeepVerificationReport,
) {
    val colors = cloudBackupVisualColors()
    val summaryItems = buildVerifiedSummaryItems(report)

    CloudBackupGlassCard(
        modifier =
            Modifier
                .fillMaxWidth()
                .padding(start = 14.dp, top = 10.dp, end = 14.dp),
        fill = colors.verifiedFill,
        border = colors.verifiedBorder,
    ) {
        Column(
            modifier = Modifier.padding(16.dp),
            verticalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            Row(
                horizontalArrangement = Arrangement.spacedBy(12.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Icon(
                    Icons.Default.Security,
                    contentDescription = null,
                    tint = colors.success,
                    modifier = Modifier.size(32.dp),
                )
                Text(
                    stringResource(R.string.cloud_backup_verification_backup_verified),
                    style = MaterialTheme.typography.titleSmall,
                    fontWeight = FontWeight.SemiBold,
                    color = colors.primaryText,
                )
            }

            VerifiedSummary(
                items = summaryItems,
                color = colors.secondaryText,
            )

            if (report.walletsFailed > 0u) {
                ErrorInlineMessage(
                    pluralStringResource(
                        R.plurals.cloud_backup_wallets_failed_decryption,
                        report.walletsFailed.toInt(),
                        report.walletsFailed.toInt(),
                    ),
                )
            }

            if (report.walletsUnsupported > 0u) {
                ErrorInlineMessage(
                    pluralStringResource(
                        R.plurals.cloud_backup_wallets_unsupported_format,
                        report.walletsUnsupported.toInt(),
                        report.walletsUnsupported.toInt(),
                    ),
                )
            }
        }
    }
}

@Composable
private fun VerifiedSummary(
    items: List<String>,
    color: Color,
) {
    if (items.size > 2) {
        Column(
            modifier = Modifier.fillMaxWidth(),
            verticalArrangement = Arrangement.spacedBy(4.dp),
        ) {
            items.forEach { item ->
                Text(
                    item,
                    style = MaterialTheme.typography.caption,
                    color = color,
                )
            }
        }

        return
    }

    Text(
        items.joinToString(" • "),
        style = MaterialTheme.typography.caption,
        color = color,
    )
}

@Composable
private fun buildVerifiedSummaryItems(report: DeepVerificationReport): List<String> =
    buildList {
        if (report.credentialRecovered) {
            add(stringResource(R.string.cloud_backup_verification_passkey_recovered))
        }
        if (report.masterKeyWrapperRepaired) {
            add(stringResource(R.string.cloud_backup_verification_master_key_repaired))
        }
        if (report.localMasterKeyRepaired) {
            add(stringResource(R.string.cloud_backup_verification_local_credentials_repaired))
        }
        add(
            pluralStringResource(
                R.plurals.cloud_backup_verified_wallets,
                report.walletsVerified.toInt(),
                report.walletsVerified.toInt(),
            ),
        )
    }

@Composable
private fun VerifyAgainButton(onClick: () -> Unit) {
    val colors = cloudBackupVisualColors()

    OutlinedButton(
        onClick = onClick,
        modifier =
            Modifier
                .fillMaxWidth()
                .heightIn(min = 48.dp)
                .padding(horizontal = 14.dp),
        shape = RoundedCornerShape(18.dp),
        border = BorderStroke(1.5.dp, colors.outlineButtonBorder),
        colors =
            ButtonDefaults.outlinedButtonColors(
                contentColor = colors.outlineButtonBorder,
            ),
    ) {
        Icon(Icons.Default.Security, contentDescription = null, modifier = Modifier.size(20.dp))
        Spacer(modifier = Modifier.width(8.dp))
        Text(
            stringResource(R.string.cloud_backup_verification_verify_again),
            style = MaterialTheme.typography.titleSmall,
            fontWeight = FontWeight.SemiBold,
        )
    }
}

@Composable
private fun VerificationFailureContent(
    failure: DeepVerificationFailure,
    manager: CloudBackupManager,
    onRecreate: () -> Unit,
    onReinitialize: () -> Unit,
) {
    when (failure) {
        is DeepVerificationFailure.Retry -> {
            ErrorInlineMessage(failure.localizedMessage().asString(), modifier = Modifier.padding(16.dp))
            MaterialDivider()
            MaterialSettingsItem(
                title = stringResource(R.string.action_try_again),
                onClick = {
                    manager.dispatch(
                        verificationRetryAction(failure),
                    )
                },
                leadingContent = { Icon(Icons.Default.Refresh, contentDescription = null) },
            )
            MaterialDivider()
            MaterialSettingsItem(
                title = stringResource(R.string.settings_action_create_new_passkey),
                onClick = { manager.dispatch(CloudBackupManagerAction.RepairPasskeyNoDiscovery) },
                leadingContent = { Icon(Icons.Default.Key, contentDescription = null) },
            )
        }

        is DeepVerificationFailure.RecreateManifest -> {
            ErrorInlineMessage(failure.localizedMessage().asString(), modifier = Modifier.padding(16.dp))
            MaterialDivider()
            MaterialSettingsItem(
                title = stringResource(R.string.cloud_backup_recreate_confirm_title),
                subtitle = failure.localizedWarning()?.asString(),
                onClick = onRecreate,
                leadingContent = { Icon(Icons.Default.Refresh, contentDescription = null) },
            )
        }

        is DeepVerificationFailure.ReinitializeBackup -> {
            ErrorInlineMessage(failure.localizedMessage().asString(), modifier = Modifier.padding(16.dp))
            MaterialDivider()
            MaterialSettingsItem(
                title = stringResource(R.string.cloud_backup_reinitialize_confirm_title),
                subtitle = failure.localizedWarning()?.asString(),
                onClick = onReinitialize,
                leadingContent = { Icon(Icons.Default.WarningAmber, contentDescription = null) },
            )
        }

        is DeepVerificationFailure.UnsupportedVersion -> {
            ErrorInlineMessage(failure.localizedMessage().asString(), modifier = Modifier.padding(16.dp))
        }
    }

    val repairState = manager.passkeyRepairState
    if (repairState == CloudBackupPasskeyRepairState.FAILED) {
        MaterialDivider()
        ErrorInlineMessage(stringResource(R.string.cloud_backup_passkey_repair_failed), modifier = Modifier.padding(16.dp))
    }
}

private fun verificationRetryAction(failure: DeepVerificationFailure.Retry): CloudBackupManagerAction =
    if (failure.retryContext?.action == CloudBackupRetryAction.VERIFY_DISCOVERABLE) {
        CloudBackupManagerAction.StartVerificationDiscoverable(
            CloudBackupVerificationSource.CLOUD_BACKUP_DETAIL,
        )
    } else {
        CloudBackupManagerAction.StartVerification(
            CloudBackupVerificationSource.CLOUD_BACKUP_DETAIL,
        )
    }
