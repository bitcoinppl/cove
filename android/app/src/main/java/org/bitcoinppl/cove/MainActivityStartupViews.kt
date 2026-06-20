package org.bitcoinppl.cove

import androidx.activity.compose.BackHandler
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.FilledTonalButton
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import org.bitcoinppl.cove_core.CatastrophicCloudRestoreResult

@Composable
internal fun CatastrophicRecoveryView(
    cloudRestoreCheck: CatastrophicCloudRestoreCheck,
    onRestoreFromCloud: () -> Unit,
    onConfirmRestoreFromCloud: () -> Unit,
    onDismissRestoreFromCloud: () -> Unit,
    onWipeLocalData: () -> Unit,
    onContactSupport: () -> Unit,
) {
    var showWipeConfirmation by remember { mutableStateOf(false) }
    val cloudRestoreResult =
        (cloudRestoreCheck as? CatastrophicCloudRestoreCheck.Complete)?.result

    BackHandler(enabled = true) {}

    Box(
        modifier = Modifier.fillMaxSize().background(Color.Black),
        contentAlignment = Alignment.Center,
    ) {
        Column(
            horizontalAlignment = Alignment.CenterHorizontally,
            modifier = Modifier.fillMaxWidth().padding(28.dp),
        ) {
            Text(
                stringResource(R.string.common_remaining_encryption_key_error_title),
                style = MaterialTheme.typography.headlineSmall,
                color = Color.White,
            )
            Spacer(modifier = Modifier.height(12.dp))
            Text(
                stringResource(R.string.common_remaining_encryption_key_error_message),
                style = MaterialTheme.typography.bodyMedium,
                color = Color.White.copy(alpha = 0.76f),
            )
            Spacer(modifier = Modifier.height(28.dp))
            FilledTonalButton(
                onClick = onRestoreFromCloud,
                enabled = cloudRestoreCheck !is CatastrophicCloudRestoreCheck.Checking,
                modifier = Modifier.fillMaxWidth(),
            ) {
                if (cloudRestoreCheck is CatastrophicCloudRestoreCheck.Checking) {
                    CircularProgressIndicator(
                        modifier = Modifier.size(18.dp),
                        strokeWidth = 2.dp,
                    )
                    Spacer(modifier = Modifier.width(8.dp))
                }
                Text(
                    if (cloudRestoreCheck is CatastrophicCloudRestoreCheck.Checking) {
                        stringResource(R.string.common_remaining_checking_cloud_backup)
                    } else {
                        stringResource(R.string.common_remaining_restore_from_cloud_backup)
                    },
                )
            }
            val failureMessage = cloudRestoreResult?.failureMessage
            if (failureMessage != null) {
                Spacer(modifier = Modifier.height(8.dp))
                Text(
                    failureMessage,
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.error,
                )
            }
            Spacer(modifier = Modifier.height(8.dp))
            TextButton(
                onClick = onContactSupport,
                modifier = Modifier.fillMaxWidth(),
            ) {
                Text(stringResource(R.string.common_remaining_contact_support))
            }
            Spacer(modifier = Modifier.height(8.dp))
            TextButton(
                onClick = { showWipeConfirmation = true },
                modifier = Modifier.fillMaxWidth(),
            ) {
                Text(stringResource(R.string.common_remaining_wipe_local_data), color = MaterialTheme.colorScheme.error)
            }
        }
    }

    if (showWipeConfirmation) {
        AlertDialog(
            onDismissRequest = { showWipeConfirmation = false },
            title = { Text(stringResource(R.string.common_remaining_wipe_local_data_title)) },
            text = {
                Text(
                    stringResource(R.string.common_remaining_wipe_local_data_message),
                )
            },
            confirmButton = {
                TextButton(
                    onClick = {
                        showWipeConfirmation = false
                        onWipeLocalData()
                    },
                ) {
                    Text(stringResource(R.string.common_remaining_wipe_data), color = MaterialTheme.colorScheme.error)
                }
            },
            dismissButton = {
                TextButton(onClick = { showWipeConfirmation = false }) { Text(stringResource(R.string.action_cancel)) }
            },
        )
    }

    if (cloudRestoreResult is CatastrophicCloudRestoreResult.BackupFound) {
        AlertDialog(
            onDismissRequest = onDismissRestoreFromCloud,
            title = { Text(stringResource(R.string.common_remaining_restore_cloud_backup_title)) },
            text = {
                Text(
                    stringResource(R.string.common_remaining_restore_cloud_backup_message),
                )
            },
            confirmButton = {
                TextButton(onClick = onConfirmRestoreFromCloud) {
                    Text(stringResource(R.string.common_remaining_erase_and_restore), color = MaterialTheme.colorScheme.error)
                }
            },
            dismissButton = {
                TextButton(onClick = onDismissRestoreFromCloud) { Text(stringResource(R.string.action_cancel)) }
            },
        )
    }
}

@Composable
internal fun BootstrapErrorView(
    errorMessage: String,
    onCopyDiagnostics: () -> Unit,
    onShareDiagnostics: () -> Unit,
) {
    Box(
        modifier = Modifier.fillMaxSize().background(Color.Black),
        contentAlignment = Alignment.Center,
    ) {
        Column(
            horizontalAlignment = Alignment.CenterHorizontally,
            modifier = Modifier.padding(16.dp),
        ) {
            Text(
                stringResource(R.string.common_remaining_storage_error_title),
                style = MaterialTheme.typography.headlineSmall,
                color = Color.White,
            )
            Spacer(modifier = Modifier.height(8.dp))
            Text(
                errorMessage,
                style = MaterialTheme.typography.bodyMedium,
                color = Color.White.copy(alpha = 0.7f),
            )
            Spacer(modifier = Modifier.height(16.dp))
            Text(
                stringResource(R.string.common_remaining_storage_error_help),
                style = MaterialTheme.typography.bodySmall,
                color = Color.White.copy(alpha = 0.5f),
            )
            Spacer(modifier = Modifier.height(12.dp))
            TextButton(onClick = onCopyDiagnostics) {
                Text(stringResource(R.string.common_remaining_copy_diagnostics), color = Color.White)
            }
            TextButton(onClick = onShareDiagnostics) {
                Text(stringResource(R.string.common_remaining_share_diagnostics), color = Color.White)
            }
        }
    }
}

@Composable
internal fun SplashLoadingView(
    showSpinner: Boolean,
    statusMessage: String? = null,
    progress: Float? = null,
) {
    Box(
        modifier = Modifier.fillMaxSize().background(Color.Black),
        contentAlignment = Alignment.Center,
    ) {
        Column(horizontalAlignment = Alignment.CenterHorizontally) {
            Image(
                painter = painterResource(id = R.drawable.cove_logo),
                contentDescription = null,
                modifier = Modifier.size(144.dp).clip(RoundedCornerShape(25.dp)),
            )
            if (showSpinner) {
                Spacer(modifier = Modifier.height(24.dp))
                CircularProgressIndicator(color = Color.White)
            }

            if (statusMessage != null) {
                Spacer(modifier = Modifier.height(12.dp))
                Text(
                    statusMessage,
                    style = MaterialTheme.typography.bodyMedium,
                    color = Color.White.copy(alpha = 0.7f),
                )
            }

            if (progress != null) {
                Spacer(modifier = Modifier.height(12.dp))
                LinearProgressIndicator(
                    progress = { progress },
                    modifier = Modifier.fillMaxWidth(0.6f),
                    color = Color.White,
                    trackColor = Color.White.copy(alpha = 0.2f),
                )
            }
        }
    }
}
