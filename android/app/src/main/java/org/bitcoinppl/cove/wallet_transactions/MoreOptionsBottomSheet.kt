package org.bitcoinppl.cove.wallet_transactions

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import org.bitcoinppl.cove.AppManager
import org.bitcoinppl.cove.AppSheetState
import org.bitcoinppl.cove.R
import org.bitcoinppl.cove.TaggedItem
import org.bitcoinppl.cove.WalletManager
import org.bitcoinppl.cove.ui.theme.CoveColor
import org.bitcoinppl.cove_core.*
import org.bitcoinppl.cove_core.types.*

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun MoreOptionsBottomSheet(
    app: AppManager,
    manager: WalletManager,
    onDismiss: () -> Unit,
) {
    val metadata = manager.walletMetadata
    val hasLabels = manager.rust.labelManager().hasLabels()
    val hasTransactions = manager.hasTransactions

    // get TapSigner if this wallet has one
    val tapSigner =
        when (val hw = metadata?.hardwareMetadata) {
            is HardwareWalletMetadata.TapSigner -> hw.v1
            else -> null
        }

    ModalBottomSheet(
        onDismissRequest = onDismiss,
        sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true),
        containerColor = Color.White,
        dragHandle = null,
        shape = RoundedCornerShape(topStart = 12.dp, topEnd = 12.dp),
    ) {
        Column(
            modifier =
                Modifier
                    .fillMaxWidth()
                    .padding(16.dp, 8.dp, 16.dp, 24.dp),
        ) {
            // drag handle
            Box(
                modifier = Modifier.fillMaxWidth(),
                contentAlignment = Alignment.Center,
            ) {
                Box(
                    modifier =
                        Modifier
                            .width(36.dp)
                            .height(4.dp)
                            .background(
                                CoveColor.BorderLight,
                                RoundedCornerShape(2.dp),
                            ),
                )
            }
            Spacer(modifier = Modifier.height(16.dp))

            // title
            Text(
                text = stringResource(R.string.title_more_options),
                color = CoveColor.TextPrimary,
                fontSize = 18.sp,
                fontWeight = FontWeight.SemiBold,
                modifier = Modifier.fillMaxWidth(),
                textAlign = androidx.compose.ui.text.style.TextAlign.Center,
            )
            Spacer(modifier = Modifier.height(24.dp))

            // menu items
            MenuItemRow(
                icon = Icons.Filled.Nfc,
                label = stringResource(R.string.menu_scan_nfc),
                onClick = {
                    // TODO: implement NFC scan
                    onDismiss()
                },
            )

            MenuItemRow(
                icon = Icons.Filled.FileDownload,
                label = stringResource(R.string.menu_import_labels),
                onClick = {
                    // TODO: implement import labels
                    onDismiss()
                },
            )

            if (hasLabels) {
                MenuItemRow(
                    icon = Icons.Filled.FileUpload,
                    label = stringResource(R.string.menu_export_labels),
                    onClick = {
                        // TODO: implement export labels
                        onDismiss()
                    },
                )
            }

            if (hasTransactions) {
                MenuItemRow(
                    icon = Icons.Filled.SwapVert,
                    label = stringResource(R.string.menu_export_transactions),
                    onClick = {
                        // TODO: implement export transactions
                        onDismiss()
                    },
                )
            }

            // TapSigner-specific options
            if (tapSigner != null) {
                MenuItemRow(
                    icon = Icons.Filled.Key,
                    label = stringResource(R.string.menu_change_pin),
                    onClick = {
                        onDismiss()
                        // open TapSigner flow with Change action
                        val route =
                            TapSignerRoute.EnterPin(
                                tapSigner = tapSigner,
                                action = AfterPinAction.Change,
                            )
                        app.sheetState = TaggedItem(AppSheetState.TapSigner(route))
                    },
                )

                MenuItemRow(
                    icon = Icons.Filled.Download,
                    label = stringResource(R.string.menu_download_backup),
                    onClick = {
                        onDismiss()
                        // check if backup already exists in cache
                        val backup = app.getTapSignerBackup(tapSigner)
                        if (backup != null) {
                            // TODO: implement export backup directly
                        } else {
                            // open TapSigner flow with Backup action
                            val route =
                                TapSignerRoute.EnterPin(
                                    tapSigner = tapSigner,
                                    action = AfterPinAction.Backup,
                                )
                            app.sheetState = TaggedItem(AppSheetState.TapSigner(route))
                        }
                    },
                )
            }

            if (hasTransactions) {
                MenuItemRow(
                    icon = Icons.Filled.Circle,
                    label = stringResource(R.string.menu_manage_utxos),
                    onClick = {
                        // TODO: implement coin control
                        onDismiss()
                    },
                )
            }

            // wallet settings - always shown last
            MenuItemRow(
                icon = Icons.Filled.Settings,
                label = stringResource(R.string.menu_wallet_settings),
                onClick = {
                    // TODO: implement wallet settings
                    onDismiss()
                },
            )
        }
    }
}

@Composable
private fun MenuItemRow(
    icon: ImageVector,
    label: String,
    onClick: () -> Unit,
) {
    Row(
        modifier =
            Modifier
                .fillMaxWidth()
                .clickable(onClick = onClick)
                .padding(vertical = 16.dp, horizontal = 8.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(16.dp),
    ) {
        Icon(
            imageVector = icon,
            contentDescription = label,
            tint = CoveColor.TextPrimary,
            modifier = Modifier.size(24.dp),
        )
        Text(
            text = label,
            color = CoveColor.TextPrimary,
            fontSize = 16.sp,
            fontWeight = FontWeight.Normal,
        )
    }
}
