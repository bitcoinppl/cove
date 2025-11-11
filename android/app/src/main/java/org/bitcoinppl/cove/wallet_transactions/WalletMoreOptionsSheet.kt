package org.bitcoinppl.cove.wallet_transactions

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Download
import androidx.compose.material.icons.filled.Key
import androidx.compose.material.icons.filled.Settings
import androidx.compose.material.icons.filled.SwapVert
import androidx.compose.material.icons.filled.Upload
import androidx.compose.material.icons.outlined.Circle
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import org.bitcoinppl.cove.AppManager
import org.bitcoinppl.cove.AppSheetState
import org.bitcoinppl.cove.TaggedItem
import org.bitcoinppl.cove.WalletManager
import org.bitcoinppl.cove.tapsigner.rememberBackupExportLauncher
import org.bitcoinppl.cove.ui.theme.CoveColor
import org.bitcoinppl.cove_core.AfterPinAction
import org.bitcoinppl.cove_core.CoinControlRoute
import org.bitcoinppl.cove_core.HardwareWalletMetadata
import org.bitcoinppl.cove_core.Route
import org.bitcoinppl.cove_core.SettingsRoute
import org.bitcoinppl.cove_core.TapSignerRoute
import org.bitcoinppl.cove_core.WalletSettingsRoute

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun WalletMoreOptionsSheet(
    app: AppManager,
    manager: WalletManager,
    onDismiss: () -> Unit,
    onImportLabels: () -> Unit,
    onExportLabels: () -> Unit,
    onExportTransactions: () -> Unit,
) {
    val metadata =
        manager.walletMetadata ?: run {
            return
        }
    val hasLabels = manager.rust.labelManager().hasLabels()
    val hasTransactions = manager.hasTransactions
    val hardwareMetadata = metadata.hardwareMetadata

    ModalBottomSheet(
        onDismissRequest = onDismiss,
        containerColor = MaterialTheme.colorScheme.surface,
    ) {
        Column(
            modifier =
                Modifier
                    .fillMaxWidth()
                    .padding(horizontal = 16.dp)
                    .padding(bottom = 32.dp),
        ) {
            // title
            Text(
                text = "More Options",
                style = MaterialTheme.typography.titleMedium,
                fontWeight = FontWeight.Bold,
                modifier =
                    Modifier
                        .fillMaxWidth()
                        .padding(vertical = 8.dp),
                textAlign = androidx.compose.ui.text.style.TextAlign.Center,
            )

            Spacer(modifier = Modifier.height(16.dp))

            // import labels
            MenuOption(
                icon = { Icon(Icons.Default.Download, contentDescription = null) },
                label = "Import Labels",
                onClick = {
                    onImportLabels()
                    onDismiss()
                },
            )

            HorizontalDivider(color = CoveColor.DividerLightAlpha.copy(alpha = 0.3f))

            // export labels (conditional)
            if (hasLabels) {
                MenuOption(
                    icon = { Icon(Icons.Default.Upload, contentDescription = null) },
                    label = "Export Labels",
                    onClick = {
                        onExportLabels()
                        onDismiss()
                    },
                )

                HorizontalDivider(color = CoveColor.DividerLightAlpha.copy(alpha = 0.3f))
            }

            // export transactions (conditional)
            if (hasTransactions) {
                MenuOption(
                    icon = { Icon(Icons.Default.SwapVert, contentDescription = null) },
                    label = "Export Transactions",
                    onClick = {
                        onExportTransactions()
                        onDismiss()
                    },
                )

                HorizontalDivider(color = CoveColor.DividerLightAlpha.copy(alpha = 0.3f))
            }

            // TapSigner options
            if (hardwareMetadata is HardwareWalletMetadata.TapSigner) {
                val tapSigner = hardwareMetadata.v1

                // launcher for creating backup file
                val createBackupLauncher =
                    rememberBackupExportLauncher(app) {
                        app.getTapSignerBackup(tapSigner)
                            ?: throw IllegalStateException("Backup not available")
                    }

                // change PIN
                MenuOption(
                    icon = { Icon(Icons.Default.Key, contentDescription = null) },
                    label = "Change PIN",
                    onClick = {
                        onDismiss()
                        val route =
                            TapSignerRoute.EnterPin(
                                tapSigner = tapSigner,
                                action = AfterPinAction.Change,
                            )
                        app.sheetState = TaggedItem(AppSheetState.TapSigner(route))
                    },
                )

                HorizontalDivider(color = CoveColor.DividerLightAlpha.copy(alpha = 0.3f))

                // download backup
                MenuOption(
                    icon = { Icon(Icons.Default.Download, contentDescription = null) },
                    label = "Download Backup",
                    onClick = {
                        onDismiss()
                        // check if backup already exists in cache
                        val backup = app.getTapSignerBackup(tapSigner)
                        if (backup != null) {
                            val fileName = "${tapSigner.identFileNamePrefix()}_backup.txt"
                            createBackupLauncher.launch(fileName)
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

                HorizontalDivider(color = CoveColor.DividerLightAlpha.copy(alpha = 0.3f))
            }

            // manage UTXOs (conditional)
            if (hasTransactions) {
                MenuOption(
                    icon = { Icon(Icons.Outlined.Circle, contentDescription = null) },
                    label = "Manage UTXOs",
                    onClick = {
                        app.pushRoute(
                            Route.CoinControl(
                                CoinControlRoute.List(metadata.id),
                            ),
                        )
                        onDismiss()
                    },
                )

                HorizontalDivider(color = CoveColor.DividerLightAlpha.copy(alpha = 0.3f))
            }

            // wallet settings (always visible)
            MenuOption(
                icon = { Icon(Icons.Default.Settings, contentDescription = null) },
                label = "Wallet Settings",
                onClick = {
                    app.pushRoute(
                        Route.Settings(
                            SettingsRoute.Wallet(
                                id = metadata.id,
                                route = WalletSettingsRoute.MAIN,
                            ),
                        ),
                    )
                    onDismiss()
                },
            )
        }
    }
}

@Composable
private fun MenuOption(
    icon: @Composable () -> Unit,
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
    ) {
        // icon
        androidx.compose.material3.Surface(
            modifier = Modifier.size(40.dp),
            color = MaterialTheme.colorScheme.surfaceVariant,
            shape = MaterialTheme.shapes.medium,
        ) {
            androidx.compose.foundation.layout.Box(
                contentAlignment = Alignment.Center,
            ) {
                icon()
            }
        }

        Spacer(modifier = Modifier.size(16.dp))

        // label
        Text(
            text = label,
            style = MaterialTheme.typography.bodyLarge,
            modifier = Modifier.weight(1f),
        )
    }
}
