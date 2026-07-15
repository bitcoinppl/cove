package org.bitcoinppl.cove.cloudbackup

import android.content.ClipData
import android.content.ClipboardManager
import android.content.Context
import android.content.Intent
import android.net.Uri
import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.CloudOff
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import org.bitcoinppl.cove.BuildConfig
import org.bitcoinppl.cove.Log
import org.bitcoinppl.cove.flows.OnboardingFlow.OnboardingBackground
import org.bitcoinppl.cove.flows.OnboardingFlow.OnboardingCardBorder
import org.bitcoinppl.cove.flows.OnboardingFlow.OnboardingCardFill
import org.bitcoinppl.cove.flows.OnboardingFlow.OnboardingStatusHero
import org.bitcoinppl.cove.flows.OnboardingFlow.OnboardingTextSecondary
import org.bitcoinppl.cove_core.CloudBackupPendingEnableCleanupState
import org.bitcoinppl.cove_core.CloudBackupPendingEnableRecovery

@Composable
internal fun CloudBackupPendingEnableRecoveryContent(
    recovery: CloudBackupPendingEnableRecovery,
    onConfirmCleanup: () -> Unit,
    onCancel: () -> Unit,
) {
    val context = LocalContext.current
    val isCleaning = recovery.cleanup == CloudBackupPendingEnableCleanupState.CLEANING
    val canRemove = recovery.cleanup == CloudBackupPendingEnableCleanupState.AVAILABLE
    var showConfirmation by remember { mutableStateOf(false) }

    OnboardingBackground {
        Column(
            modifier =
                Modifier
                    .fillMaxSize()
                    .statusBarsPadding()
                    .verticalScroll(rememberScrollState())
                    .padding(horizontal = 24.dp, vertical = 18.dp),
            verticalArrangement = Arrangement.spacedBy(24.dp),
        ) {
            Row(modifier = Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.End) {
                Text(
                    text = "Cancel",
                    color = Color.White,
                    style = MaterialTheme.typography.titleMedium,
                    fontWeight = FontWeight.SemiBold,
                    modifier =
                        Modifier
                            .clickable(enabled = !isCleaning, onClick = onCancel)
                            .padding(horizontal = 8.dp, vertical = 4.dp),
                )
            }

            OnboardingStatusHero(icon = Icons.Default.CloudOff)

            Column(verticalArrangement = Arrangement.spacedBy(12.dp)) {
                Text(
                    text = "Cloud Backup Needs Recovery",
                    color = Color.White,
                    fontSize = 38.sp,
                    lineHeight = 42.sp,
                    fontWeight = FontWeight.SemiBold,
                )
                Text(
                    text = "Cloud Backup setup was interrupted and its local recovery records do not match.",
                    color = OnboardingTextSecondary,
                    style = MaterialTheme.typography.bodySmall,
                )
            }

            RecoveryCard {
                Text(
                    text =
                        if (canRemove) {
                            "Cove verified that the incomplete local setup can be removed without changing your active backup or cloud data."
                        } else {
                            "Contact support and include the code below. Don’t change Cloud Backup settings until the recovery state has been reviewed."
                        },
                    color = Color.White.copy(alpha = 0.85f),
                    style = MaterialTheme.typography.bodyMedium,
                )
            }

            RecoveryCard {
                Text(
                    text = "Support code",
                    color = OnboardingTextSecondary,
                    style = MaterialTheme.typography.labelMedium,
                )
                Text(
                    text = recovery.supportCode,
                    color = Color.White,
                    fontFamily = FontFamily.Monospace,
                    fontWeight = FontWeight.SemiBold,
                    style = MaterialTheme.typography.titleMedium,
                    modifier = Modifier.testTag("cloudBackup.recovery.supportCode"),
                )
                Row(
                    modifier = Modifier.fillMaxWidth(),
                    horizontalArrangement = Arrangement.spacedBy(12.dp),
                ) {
                    TextButton(
                        onClick = { copySupportCode(context, recovery.supportCode) },
                        modifier = Modifier.weight(1f),
                    ) {
                        Text("Copy Code")
                    }
                    TextButton(
                        onClick = { contactSupport(context, recovery.supportCode) },
                        modifier = Modifier.weight(1f),
                    ) {
                        Text("Contact Support")
                    }
                }
            }

            when {
                isCleaning -> {
                    Row(
                        modifier = Modifier.fillMaxWidth().padding(vertical = 16.dp),
                        horizontalArrangement = Arrangement.Center,
                        verticalAlignment = Alignment.CenterVertically,
                    ) {
                        CircularProgressIndicator(modifier = Modifier.size(24.dp))
                        Spacer(modifier = Modifier.size(12.dp))
                        Text("Removing incomplete setup...", color = Color.White)
                    }
                }

                canRemove -> {
                    TextButton(
                        onClick = { showConfirmation = true },
                        modifier =
                            Modifier
                                .fillMaxWidth()
                                .testTag("cloudBackup.recovery.removeIncompleteSetup"),
                    ) {
                        Text("Remove Incomplete Setup", color = MaterialTheme.colorScheme.error)
                    }
                }
            }
        }
    }

    if (showConfirmation) {
        AlertDialog(
            onDismissRequest = { showConfirmation = false },
            title = { Text("Remove incomplete Cloud Backup setup?") },
            text = {
                Text(
                    "This removes only local data from the interrupted setup. Your active Cloud Backup key, cloud data, and wallets on this device will be preserved.",
                )
            },
            confirmButton = {
                TextButton(
                    onClick = {
                        showConfirmation = false
                        onConfirmCleanup()
                    },
                    modifier = Modifier.testTag("cloudBackup.recovery.confirmRemoveIncompleteSetup"),
                ) {
                    Text("Remove Incomplete Setup", color = MaterialTheme.colorScheme.error)
                }
            },
            dismissButton = {
                TextButton(onClick = { showConfirmation = false }) { Text("Cancel") }
            },
        )
    }
}

@Composable
private fun RecoveryCard(content: @Composable ColumnScope.() -> Unit) {
    Surface(
        shape = RoundedCornerShape(10.dp),
        color = OnboardingCardFill,
        border = BorderStroke(1.dp, OnboardingCardBorder),
    ) {
        Column(
            modifier = Modifier.fillMaxWidth().padding(16.dp),
            verticalArrangement = Arrangement.spacedBy(12.dp),
            content = content,
        )
    }
}

private fun copySupportCode(context: Context, supportCode: String) {
    val clipboard = context.getSystemService(ClipboardManager::class.java)
    clipboard.setPrimaryClip(ClipData.newPlainText("Cove Cloud Backup support code", supportCode))
}

private fun contactSupport(context: Context, supportCode: String) {
    val subject = Uri.encode("Cove Cloud Backup recovery $supportCode")
    val body = Uri.encode(
        "Support code: $supportCode\nPlatform: Android\nApp version: ${BuildConfig.VERSION_NAME}",
    )
    val intent = Intent(Intent.ACTION_SENDTO).apply {
        data = Uri.parse("mailto:feedback@covebitcoinwallet.com?subject=$subject&body=$body")
    }

    runCatching { context.startActivity(intent) }.onFailure { error ->
        Log.w("CloudBackupRecovery", "failed to open support email", error)
    }
}
