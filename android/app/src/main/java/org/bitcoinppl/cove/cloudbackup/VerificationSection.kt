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
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
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
internal fun CancelledVerificationRecoveryContent(
    manager: CloudBackupManager,
) {
    Column(modifier = Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(16.dp)) {
        ErrorStateCard(
            icon = Icons.Default.WarningAmber,
            title = "Cloud Backup Not Verified",
            body = "If your passkey was deleted, add a new one. Otherwise, verify again with your current passkey.",
        )

        SectionHeader("Verification")
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
        title = "Passkey verified",
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
                    title = "Verify Now",
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
                    title = "Verifying backup integrity",
                    message = "Confirming that wallet backups can be decrypted and restored",
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

            CloudBackupVerificationState.Cancelled -> {
                CancelledVerificationRecoveryContent(manager)
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

        when (val sync = manager.syncState) {
            is CloudBackupSyncState.Failed -> {
                ErrorInlineMessage(sync.v1, modifier = Modifier.padding(horizontal = 14.dp))
            }

            is CloudBackupSyncState.Blocked -> {
                ErrorInlineMessage(sync.v1, modifier = Modifier.padding(horizontal = 14.dp))
            }

            else -> Unit
        }

        val needsSync = manager.detail?.needsSync?.isNotEmpty() == true
        if (needsSync) {
            CloudBackupSimpleActionCard(
                title = "Sync Now",
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
        title = "Verify Now",
        subtitle = "Try again with your current passkey",
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
        title = "Add New Passkey",
        subtitle = "Use this if your previous passkey was deleted",
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
                    "Backup verified",
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
                ErrorInlineMessage("${report.walletsFailed} wallet backup(s) could not be decrypted")
            }

            if (report.walletsUnsupported > 0u) {
                ErrorInlineMessage("${report.walletsUnsupported} wallet(s) use a newer backup format")
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

private fun buildVerifiedSummaryItems(report: DeepVerificationReport): List<String> =
    buildList {
        if (report.credentialRecovered) {
            add("passkey recovered")
        }
        if (report.masterKeyWrapperRepaired) {
            add("cloud master key protection repaired")
        }
        if (report.localMasterKeyRepaired) {
            add("local backup credentials repaired")
        }
        add("${report.walletsVerified} wallet(s) verified")
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
            "Verify Again",
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
    Column {
        when (failure) {
            is DeepVerificationFailure.Retry -> {
                ErrorInlineMessage(failure.message, modifier = Modifier.padding(16.dp))
                MaterialDivider()
                MaterialSettingsItem(
                    title = "Try Again",
                    onClick = {
                        manager.dispatch(
                            verificationRetryAction(failure),
                        )
                    },
                    leadingContent = { Icon(Icons.Default.Refresh, contentDescription = null) },
                )
                MaterialDivider()
                MaterialSettingsItem(
                    title = "Create New Passkey",
                    onClick = { manager.dispatch(CloudBackupManagerAction.RepairPasskeyNoDiscovery) },
                    leadingContent = { Icon(Icons.Default.Key, contentDescription = null) },
                )
            }

            is DeepVerificationFailure.RecreateManifest -> {
                ErrorInlineMessage(failure.message, modifier = Modifier.padding(16.dp))
                MaterialDivider()
                MaterialSettingsItem(
                    title = "Recreate Backup Index",
                    subtitle = failure.warning,
                    onClick = onRecreate,
                    leadingContent = { Icon(Icons.Default.Refresh, contentDescription = null) },
                )
            }

            is DeepVerificationFailure.ReinitializeBackup -> {
                ErrorInlineMessage(failure.message, modifier = Modifier.padding(16.dp))
                MaterialDivider()
                MaterialSettingsItem(
                    title = "Reinitialize Cloud Backup",
                    subtitle = failure.warning,
                    onClick = onReinitialize,
                    leadingContent = { Icon(Icons.Default.WarningAmber, contentDescription = null) },
                )
            }

            is DeepVerificationFailure.UnsupportedVersion -> {
                ErrorInlineMessage(failure.message, modifier = Modifier.padding(16.dp))
            }
        }

        val repairState = manager.passkeyRepairState
        if (repairState is CloudBackupPasskeyRepairState.Failed) {
            MaterialDivider()
            ErrorInlineMessage(repairState.v1, modifier = Modifier.padding(16.dp))
        }
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
