package org.bitcoinppl.cove.cloudbackup

import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.KeyboardArrowRight
import androidx.compose.material.icons.automirrored.filled.Label
import androidx.compose.material.icons.filled.AccountBalanceWallet
import androidx.compose.material.icons.filled.CalendarToday
import androidx.compose.material.icons.filled.Check
import androidx.compose.material.icons.filled.CloudDone
import androidx.compose.material.icons.filled.CloudOff
import androidx.compose.material.icons.filled.CloudUpload
import androidx.compose.material.icons.filled.DoNotDisturbOn
import androidx.compose.material.icons.filled.Refresh
import androidx.compose.material.icons.filled.Schedule
import androidx.compose.material.icons.filled.WarningAmber
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.res.pluralStringResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import org.bitcoinppl.cove.R
import org.bitcoinppl.cove.UiText
import org.bitcoinppl.cove.localizedDisplayText
import org.bitcoinppl.cove.ui.theme.caption
import org.bitcoinppl.cove_core.CloudBackupWalletItem
import org.bitcoinppl.cove_core.CloudBackupWalletStatus
import org.bitcoinppl.cove_core.WalletMode
import org.bitcoinppl.cove_core.WalletType
import org.bitcoinppl.cove_core.device.CloudSyncHealth
import org.bitcoinppl.cove_core.types.Network

@Composable
private fun WalletRowsSection(
    title: String,
    wallets: List<CloudBackupWalletItem>,
    modifier: Modifier = Modifier,
    icon: ImageVector? = null,
    tint: Color? = null,
    bitcoinIcon: Boolean = false,
    onWalletClick: ((CloudBackupWalletItem) -> Unit)? = null,
    showChevron: Boolean = onWalletClick != null,
    operatingRecordId: String? = null,
    rowsEnabled: Boolean = true,
) {
    CloudBackupTitledContentSection(
        title = title,
        modifier = modifier,
        icon = icon,
        tint = tint,
        bitcoinIcon = bitcoinIcon,
    ) {
        WalletRowsCard(
            wallets = wallets,
            onWalletClick = onWalletClick,
            showChevron = showChevron,
            operatingRecordId = operatingRecordId,
            rowsEnabled = rowsEnabled,
        )
    }
}

@Composable
internal fun WalletRowsCard(
    wallets: List<CloudBackupWalletItem>,
    onWalletClick: ((CloudBackupWalletItem) -> Unit)? = null,
    showChevron: Boolean = onWalletClick != null,
    operatingRecordId: String? = null,
    rowsEnabled: Boolean = true,
) {
    val colors = cloudBackupVisualColors()

    CloudBackupGlassCard(
        modifier =
            Modifier
                .fillMaxWidth()
                .padding(horizontal = 14.dp),
    ) {
        Column {
            wallets.forEachIndexed { index, item ->
                val isOperating = operatingRecordId == item.recordId

                WalletItemRow(
                    item = item,
                    onClick = onWalletClick?.let { onClick -> { onClick(item) } },
                    showChevron = showChevron,
                    isOperating = isOperating,
                    enabled = rowsEnabled,
                )
                if (index != wallets.lastIndex) {
                    HorizontalDivider(
                        color = colors.divider,
                        modifier = Modifier.padding(horizontal = 14.dp),
                    )
                }
            }
        }
    }
}

@Composable
internal fun CloudBackupHeaderSection(
    lastSync: ULong?,
    syncHealth: CloudSyncHealth,
) {
    val colors = cloudBackupVisualColors()
    val title = cloudBackupHeaderTitle(syncHealth)

    val (icon, tint, label) =
        when (syncHealth) {
            CloudSyncHealth.UNKNOWN ->
                Triple(Icons.Default.CloudOff, colors.secondaryText, stringResource(R.string.cloud_backup_health_checking))
            CloudSyncHealth.ALL_UPLOADED ->
                Triple(Icons.Default.CloudDone, colors.success, stringResource(R.string.cloud_backup_health_all_confirmed))
            CloudSyncHealth.UPLOADING ->
                Triple(Icons.Default.CloudUpload, colors.cloudBlue, stringResource(R.string.cloud_backup_health_uploading))
            CloudSyncHealth.FAILED ->
                Triple(Icons.Default.WarningAmber, colors.danger, stringResource(R.string.cloud_backup_health_failed))
            CloudSyncHealth.NO_FILES ->
                Triple(Icons.Default.CloudOff, colors.secondaryText, stringResource(R.string.cloud_backup_health_no_files))
            CloudSyncHealth.AUTHORIZATION_REQUIRED ->
                Triple(
                    Icons.Default.WarningAmber,
                    colors.danger,
                    stringResource(R.string.cloud_backup_health_authorization_required),
                )
            CloudSyncHealth.UNAVAILABLE ->
                Triple(Icons.Default.CloudOff, colors.secondaryText, stringResource(R.string.cloud_backup_health_unavailable))
        }

    CloudBackupGlassCard(
        modifier =
            Modifier
                .fillMaxWidth()
                .padding(horizontal = 14.dp, vertical = 12.dp),
        fill = colors.elevatedCardFill,
        border = colors.cardBorder,
    ) {
        Row(
            modifier =
                Modifier
                    .fillMaxWidth()
                    .padding(16.dp),
            horizontalArrangement = Arrangement.spacedBy(12.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            CloudBackupIconBubble(
                icon = icon,
                fill = colors.cloudBlueFill,
                tint = colors.cloudBlue,
                size = 48.dp,
                iconSize = 28.dp,
            )

            Column(
                modifier = Modifier.weight(1f),
                verticalArrangement = Arrangement.spacedBy(7.dp),
            ) {
                Text(
                    title,
                    style = MaterialTheme.typography.bodyLarge,
                    fontWeight = FontWeight.SemiBold,
                    color = colors.primaryText,
                )

                lastSync?.let {
                    CloudBackupIconText(
                        icon = Icons.Default.Schedule,
                        text = stringResource(R.string.cloud_backup_last_synced, cloudBackupFormattedDate(it)),
                        color = colors.secondaryText,
                        iconSize = 14.dp,
                        textStyle = MaterialTheme.typography.caption,
                    )
                }

                Row(
                    horizontalArrangement = Arrangement.spacedBy(8.dp),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    if (syncHealth == CloudSyncHealth.UPLOADING) {
                        CircularProgressIndicator(
                            modifier = Modifier.size(18.dp),
                            color = colors.cloudBlue,
                            strokeWidth = 2.dp,
                        )
                    } else {
                        Icon(icon, contentDescription = null, tint = tint, modifier = Modifier.size(22.dp))
                    }
                    Text(
                        label,
                        style = MaterialTheme.typography.caption,
                        color = tint,
                        maxLines = 2,
                        overflow = TextOverflow.Ellipsis,
                    )
                }
            }
        }
    }
}

@Composable
internal fun cloudBackupHeaderTitle(syncHealth: CloudSyncHealth): String =
    cloudBackupHeaderTitleText(syncHealth).asString()

internal fun cloudBackupHeaderTitleText(syncHealth: CloudSyncHealth): UiText =
    when (syncHealth) {
        CloudSyncHealth.ALL_UPLOADED -> UiText.resource(R.string.cloud_backup_header_active)
        CloudSyncHealth.UPLOADING -> UiText.resource(R.string.cloud_backup_header_syncing)
        CloudSyncHealth.UNKNOWN -> UiText.resource(R.string.cloud_backup_header_checking)
        CloudSyncHealth.NO_FILES,
        CloudSyncHealth.AUTHORIZATION_REQUIRED,
        CloudSyncHealth.UNAVAILABLE,
        CloudSyncHealth.FAILED,
        -> UiText.resource(R.string.cloud_backup_header_attention)
    }

@Composable
internal fun WalletSections(
    title: String,
    wallets: List<CloudBackupWalletItem>,
) {
    val upToDateTitle = stringResource(R.string.cloud_backup_wallet_section_up_to_date)
    val bitcoinTitle = Network.BITCOIN.localizedDisplayText().asString()
    val unsupportedTitle = stringResource(R.string.cloud_backup_wallet_unsupported_network)
    val grouped =
        wallets
            .groupBy { GroupKey(it.network?.localizedDisplayText()?.asString() ?: unsupportedTitle, it.walletMode) }
            .toSortedMap()

    Column(verticalArrangement = Arrangement.spacedBy(CloudBackupSectionTitleContentSpacing)) {
        grouped.forEach { (group, items) ->
            val groupTitle = group.localizedTitle()
            val sectionTitle =
                if (title == upToDateTitle) {
                    groupTitle
                } else {
                    stringResource(R.string.cloud_backup_wallet_group_with_status, groupTitle, title)
                }
            WalletRowsSection(
                title = sectionTitle,
                wallets = items,
                icon = if (group.network == bitcoinTitle) null else Icons.Default.AccountBalanceWallet,
                bitcoinIcon = group.network == bitcoinTitle,
            )
        }
    }
}

private data class GroupKey(
    val network: String,
    val walletMode: WalletMode?,
) : Comparable<GroupKey> {
    override fun compareTo(other: GroupKey): Int =
        compareValuesBy(this, other, GroupKey::network, { it.walletMode?.ordinal ?: Int.MAX_VALUE })
}

@Composable
private fun GroupKey.localizedTitle(): String =
    if (walletMode == WalletMode.DECOY) {
        stringResource(R.string.cloud_backup_wallet_group_decoy, network)
    } else {
        network
    }

@Composable
private fun WalletItemRow(
    item: CloudBackupWalletItem,
    onClick: (() -> Unit)? = null,
    showChevron: Boolean = false,
    isOperating: Boolean = false,
    enabled: Boolean = true,
) {
    val colors = cloudBackupVisualColors()
    val primaryMetadata =
        buildList {
            item.network?.localizedDisplayText()?.asString()?.let(::add)
            item.walletType?.localizedDisplayText()?.asString()?.let(::add)
            item.fingerprint?.let(::add)
        }.joinToString(" • ")
    val labelCount = (item.labelCount ?: 0u).toLong().coerceAtMost(Int.MAX_VALUE.toLong()).toInt()
    val labelText = pluralStringResource(R.plurals.cloud_backup_label_count, labelCount, labelCount)
    val updatedAt = item.backupUpdatedAt?.let(::cloudBackupFormattedDate)
    val shape = RoundedCornerShape(18.dp)

    Row(
        modifier =
            Modifier
                .fillMaxWidth()
                .clip(shape)
                .then(
                    if (onClick != null) {
                        Modifier.clickable(enabled = enabled, onClick = onClick)
                    } else {
                        Modifier
                    },
                )
                .padding(horizontal = 14.dp, vertical = 12.dp),
        horizontalArrangement = Arrangement.spacedBy(10.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        if (isOperating) {
            CircularProgressIndicator(
                modifier = Modifier.size(22.dp),
                color = colors.cloudBlue,
                strokeWidth = 2.5.dp,
            )
        }

        Column(
            modifier = Modifier.weight(1f),
            verticalArrangement = Arrangement.spacedBy(4.dp),
        ) {
            Row(
                modifier = Modifier.fillMaxWidth(),
                horizontalArrangement = Arrangement.spacedBy(8.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Text(
                    item.name,
                    modifier = Modifier.weight(1f),
                    style = MaterialTheme.typography.bodyLarge,
                    fontWeight = FontWeight.SemiBold,
                    color = colors.primaryText,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                )
                StatusBadge(status = item.syncStatus)
                if (showChevron) {
                    Icon(
                        Icons.AutoMirrored.Default.KeyboardArrowRight,
                        contentDescription = null,
                        tint = colors.secondaryText,
                        modifier = Modifier.size(22.dp),
                    )
                } else if (item.syncStatus == CloudBackupWalletStatus.UNSUPPORTED_VERSION) {
                    Icon(Icons.Default.WarningAmber, contentDescription = null, tint = colors.warning)
                }
            }
            CloudBackupBitcoinMetadataText(primaryMetadata)
            Row(
                modifier = Modifier.fillMaxWidth(),
                horizontalArrangement = Arrangement.spacedBy(8.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                CloudBackupIconText(
                    icon = Icons.AutoMirrored.Default.Label,
                    text = labelText,
                    color = colors.secondaryText,
                    maxLines = 1,
                    modifier = Modifier.widthIn(max = 70.dp),
                )
                updatedAt?.let {
                    Text("•", color = colors.secondaryText, style = MaterialTheme.typography.caption)
                    CloudBackupIconText(
                        icon = Icons.Default.CalendarToday,
                        text = it,
                        color = colors.secondaryText,
                        maxLines = 1,
                        modifier = Modifier.weight(1f),
                    )
                }
            }
        }
    }
}

@Composable
private fun StatusBadge(
    status: CloudBackupWalletStatus,
) {
    val colors = cloudBackupVisualColors()
    val (label, color, fill, border, icon) =
        when (status) {
            CloudBackupWalletStatus.DIRTY -> StatusBadgeStyle(stringResource(R.string.cloud_backup_wallet_status_dirty), colors.warning, colors.warningFill, colors.warningBorder, Icons.Default.WarningAmber)
            CloudBackupWalletStatus.UPLOADING,
            CloudBackupWalletStatus.UPLOADED_PENDING_CONFIRMATION,
            -> StatusBadgeStyle(stringResource(R.string.cloud_backup_wallet_status_syncing), colors.cloudBlue, colors.cloudBlueFill, colors.cloudBlue.copy(alpha = 0.48f), Icons.Default.Refresh)
            CloudBackupWalletStatus.CONFIRMED -> StatusBadgeStyle(stringResource(R.string.cloud_backup_wallet_status_confirmed), colors.success, colors.successFill, colors.successBorder, Icons.Default.Check)
            CloudBackupWalletStatus.FAILED -> StatusBadgeStyle(stringResource(R.string.cloud_backup_wallet_status_failed), colors.danger, colors.dangerFill, colors.dangerBorder, Icons.Default.WarningAmber)
            CloudBackupWalletStatus.DELETED_FROM_DEVICE -> StatusBadgeStyle(stringResource(R.string.cloud_backup_wallet_status_not_on_device), colors.warning, colors.warningFill, colors.warningBorder, Icons.Default.DoNotDisturbOn)
            CloudBackupWalletStatus.UNSUPPORTED_VERSION -> StatusBadgeStyle(stringResource(R.string.cloud_backup_wallet_status_unsupported), colors.warning, colors.warningFill, colors.warningBorder, Icons.Default.WarningAmber)
            CloudBackupWalletStatus.REMOTE_STATE_UNKNOWN -> StatusBadgeStyle(stringResource(R.string.cloud_backup_wallet_status_unknown), colors.secondaryText, colors.cardFill, colors.cardBorder, Icons.Default.WarningAmber)
        }

    Surface(
        color = fill,
        shape = CircleShape,
        border = BorderStroke(1.dp, border),
    ) {
        Row(
            modifier = Modifier.padding(horizontal = 7.dp, vertical = 4.dp),
            horizontalArrangement = Arrangement.spacedBy(4.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Icon(icon, contentDescription = null, tint = color, modifier = Modifier.size(12.dp))
            Text(
                label,
                style = MaterialTheme.typography.caption,
                color = color,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
            )
        }
    }
}

private data class StatusBadgeStyle(
    val label: String,
    val color: Color,
    val fill: Color,
    val border: Color,
    val icon: ImageVector,
)
