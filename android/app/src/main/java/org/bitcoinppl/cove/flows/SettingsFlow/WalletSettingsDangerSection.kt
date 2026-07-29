package org.bitcoinppl.cove.flows.SettingsFlow

import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.size
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Delete
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import org.bitcoinppl.cove.AppManager
import org.bitcoinppl.cove.AuthManager
import org.bitcoinppl.cove.R
import org.bitcoinppl.cove.WalletManager
import org.bitcoinppl.cove.ui.theme.CoveColor
import org.bitcoinppl.cove.views.MaterialDivider
import org.bitcoinppl.cove.views.MaterialSection
import org.bitcoinppl.cove.views.MaterialSettingsItem
import org.bitcoinppl.cove.views.SectionHeader
import org.bitcoinppl.cove_core.Route
import org.bitcoinppl.cove_core.WalletMetadata
import org.bitcoinppl.cove_core.WalletType

@Composable
internal fun WalletSettingsDangerSection(
    app: AppManager,
    manager: WalletManager,
    metadata: WalletMetadata,
    auth: AuthManager,
    onExportPrivateKey: () -> Unit,
    onDeleteClick: () -> Unit,
) {
    SectionHeader(stringResource(R.string.title_wallet_danger_zone))
    MaterialSection {
        Column {
            var dangerItemCount = 0
            if (metadata.walletType == WalletType.HOT) {
                if (manager.hasRecoveryWords()) {
                    MaterialSettingsItem(
                        title = stringResource(R.string.label_wallet_view_secrets),
                        titleColor = CoveColor.WarningOrange,
                        onClick = {
                            app.pushRoute(Route.SecretWords(metadata.id))
                        },
                    )
                    dangerItemCount++
                } else if (manager.hasXprvSecret() && !auth.isInDecoyMode()) {
                    MaterialSettingsItem(
                        title = stringResource(R.string.label_wallet_export_private_key),
                        titleColor = CoveColor.WarningOrange,
                        onClick = onExportPrivateKey,
                    )
                    dangerItemCount++
                }
            }
            if (dangerItemCount > 0) MaterialDivider()
            MaterialSettingsItem(
                title = stringResource(R.string.label_wallet_delete),
                titleColor = MaterialTheme.colorScheme.error,
                leadingContent = {
                    Icon(
                        imageVector = Icons.Default.Delete,
                        contentDescription = null,
                        tint = MaterialTheme.colorScheme.error,
                        modifier = Modifier.size(24.dp),
                    )
                },
                onClick = onDeleteClick,
            )
        }
    }
}

@Composable
internal fun WalletSettingsDeleteDialogs(
    walletName: String,
    firstDeleteMessage: String,
    finalDeleteMessage: String,
    finalDeleteButtonTitle: String,
    requiredConfirmations: UByte,
    showFirstDeleteConfirmation: Boolean,
    showSecondDeleteConfirmation: Boolean,
    showFinalDeleteConfirmation: Boolean,
    deleteError: String?,
    onDismissFirst: () -> Unit,
    onDismissSecond: () -> Unit,
    onDismissFinal: () -> Unit,
    onDismissError: () -> Unit,
    onRequestSecond: () -> Unit,
    onRequestFinal: () -> Unit,
    onDelete: () -> Unit,
) {
    if (showFirstDeleteConfirmation) {
        AlertDialog(
            onDismissRequest = onDismissFirst,
            title = { Text("Are you sure?") },
            text = { Text(firstDeleteMessage) },
            confirmButton = {
                TextButton(
                    onClick = {
                        onDismissFirst()
                        if (requiredConfirmations >= 2u) {
                            onRequestSecond()
                        } else {
                            onDelete()
                        }
                    },
                ) {
                    Text("Delete", color = MaterialTheme.colorScheme.error)
                }
            },
            dismissButton = {
                TextButton(onClick = onDismissFirst) {
                    Text("Cancel")
                }
            },
        )
    }

    if (showSecondDeleteConfirmation) {
        AlertDialog(
            onDismissRequest = onDismissSecond,
            title = { Text("Confirm Deletion") },
            text = { Text("Are you sure you want to delete '$walletName'?") },
            confirmButton = {
                TextButton(
                    onClick = {
                        onDismissSecond()
                        if (requiredConfirmations >= 3u) {
                            onRequestFinal()
                        } else {
                            onDelete()
                        }
                    },
                ) {
                    Text("Delete", color = MaterialTheme.colorScheme.error)
                }
            },
            dismissButton = {
                TextButton(onClick = onDismissSecond) {
                    Text("Cancel")
                }
            },
        )
    }

    if (showFinalDeleteConfirmation) {
        AlertDialog(
            onDismissRequest = onDismissFinal,
            title = { Text("Final Warning") },
            text = { Text(finalDeleteMessage) },
            confirmButton = {
                TextButton(onClick = onDelete) {
                    Text(finalDeleteButtonTitle, color = MaterialTheme.colorScheme.error)
                }
            },
            dismissButton = {
                TextButton(onClick = onDismissFinal) {
                    Text("Cancel")
                }
            },
        )
    }

    deleteError?.let { error ->
        AlertDialog(
            onDismissRequest = onDismissError,
            title = { Text("Failed to Delete Wallet") },
            text = { Text(error) },
            confirmButton = {
                TextButton(onClick = onDismissError) {
                    Text("OK")
                }
            },
        )
    }
}
