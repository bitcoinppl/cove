package org.bitcoinppl.cove.cloudbackup

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.CloudDone
import androidx.compose.material.icons.filled.CloudUpload
import androidx.compose.material.icons.filled.ErrorOutline
import androidx.compose.material.icons.filled.Key
import androidx.compose.material3.Button
import androidx.compose.material3.Card
import androidx.compose.material3.CardDefaults
import androidx.compose.material3.Icon
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
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import org.bitcoinppl.cove.ui.theme.coveColors
import org.bitcoinppl.cove_core.CloudBackupEnableFlow
import org.bitcoinppl.cove_core.CloudBackupManagerAction
import org.bitcoinppl.cove_core.SavedPasskeyConfirmationMode

@Composable
internal fun CloudBackupEnableProgressOrConfirmation(manager: CloudBackupManager) {
    val enableFlow = manager.enableFlow
    if (enableFlow is CloudBackupEnableFlow.AwaitingSavedPasskeyConfirmation &&
        enableFlow.v1 == SavedPasskeyConfirmationMode.MANUAL
    ) {
        CloudBackupPasskeyConfirmationContent(
            onContinue = { manager.dispatch(CloudBackupManagerAction.ConfirmSavedPasskey) },
            onCancel = { manager.dispatch(CloudBackupManagerAction.DiscardPendingEnableCloudBackup) },
        )
        return
    }

    val (title, message) = cloudBackupEnableProgressCopy(enableFlow)
    CloudBackupProgressContent(title = title, message = message)
}

private fun cloudBackupEnableProgressCopy(enableFlow: CloudBackupEnableFlow?): Pair<String, String> =
    when (enableFlow) {
        CloudBackupEnableFlow.CreatingPasskey ->
            "Creating your passkey..." to "Cloud Backup will continue automatically"
        CloudBackupEnableFlow.WaitingForPasskeyAvailability ->
            "Checking that your passkey is available..." to
                "This can take a few seconds after saving it in your passkey/password manager app"
        is CloudBackupEnableFlow.AwaitingSavedPasskeyConfirmation ->
            "Checking that your passkey is available..." to
                "This can take a few seconds after saving it in your passkey/password manager app"
        CloudBackupEnableFlow.ConfirmingSavedPasskey ->
            "Confirming your passkey..." to "Cloud Backup will continue automatically"
        is CloudBackupEnableFlow.UploadingInitialBackup,
        is CloudBackupEnableFlow.RetryingUploadWithStagedMaterial,
        ->
            "Creating your encrypted backup..." to "Cloud Backup will continue automatically"
        is CloudBackupEnableFlow.AwaitingForceNewConfirmation,
        is CloudBackupEnableFlow.AwaitingPasskeyChoice,
        CloudBackupEnableFlow.DiscoveringExistingBackup,
        null,
        -> "Creating your encrypted backup..." to "Cloud Backup will continue automatically"
    }

@Composable
private fun CloudBackupPasskeyConfirmationContent(
    onContinue: () -> Unit,
    onCancel: () -> Unit,
) {
    Column(
        modifier =
            Modifier
                .fillMaxSize()
                .padding(24.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.Center,
    ) {
        Icon(Icons.Default.Key, contentDescription = null, tint = MaterialTheme.colorScheme.primary)
        Spacer(modifier = Modifier.height(16.dp))
        Text("Confirm your passkey", style = MaterialTheme.typography.titleLarge)
        Spacer(modifier = Modifier.height(12.dp))
        Text(
            "Your passkey was saved. Cove needs to confirm it once before enabling Cloud Backup. If it does not appear right away, use the option to search your passkey/password manager app.",
            style = MaterialTheme.typography.bodyMedium,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
        Spacer(modifier = Modifier.height(24.dp))
        Button(onClick = onContinue, modifier = Modifier.fillMaxWidth()) {
            Text("Continue")
        }
        TextButton(onClick = onCancel, modifier = Modifier.fillMaxWidth()) {
            Text("Cancel")
        }
    }
}

@Composable
internal fun CloudBackupEnableContent(
    modifier: Modifier,
    message: String?,
    isBusy: Boolean,
    onEnable: () -> Unit,
) {
    var understandPasskey by remember { mutableStateOf(false) }
    var understandAccount by remember { mutableStateOf(false) }
    var understandManualBackup by remember { mutableStateOf(false) }
    val infoColor = MaterialTheme.colorScheme.primary

    val allChecked = understandPasskey && understandAccount && understandManualBackup

    Column(
        modifier =
            modifier
                .verticalScroll(rememberScrollState())
                .padding(24.dp),
        verticalArrangement = Arrangement.spacedBy(20.dp),
    ) {
        Spacer(modifier = Modifier.height(8.dp))

        Surface(
            color = infoColor.copy(alpha = 0.08f),
            shape = CircleShape,
            modifier = Modifier.align(Alignment.CenterHorizontally),
        ) {
            Icon(
                imageVector = Icons.Default.CloudUpload,
                contentDescription = null,
                tint = infoColor,
                modifier = Modifier.padding(24.dp),
            )
        }

        Text("Cloud Backup", style = MaterialTheme.typography.headlineMedium)
        Text(
            "Cloud Backup is end-to-end encrypted before it leaves your device and stored in Google Drive app data, secured by a passkey that only you control.",
            style = MaterialTheme.typography.bodyLarge,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )

        CloudBackupInfoCard(
            title = "How it works",
            body = "Your wallet backup is encrypted on-device, stored in your Google Drive app data, and protected by a passkey. Both your Google account and your passkey are required to restore it.",
        )

        message?.let {
            ErrorInlineMessage(it)
        }

        CloudBackupChecklistRow(
            checked = understandPasskey,
            title = "I understand that my passkey is required to access Cloud Backup and I should not delete it",
            onCheckedChange = { understandPasskey = it },
        )
        CloudBackupChecklistRow(
            checked = understandAccount,
            title = "I understand that I need access to my Google account and my passkey or this backup will not be recoverable",
            onCheckedChange = { understandAccount = it },
        )
        CloudBackupChecklistRow(
            checked = understandManualBackup,
            title = "I understand that I should still keep my 12 or 24 words backed up offline on paper",
            onCheckedChange = { understandManualBackup = it },
        )

        Button(
            onClick = onEnable,
            enabled = allChecked && !isBusy,
            modifier = Modifier.fillMaxWidth(),
        ) {
            Text("Enable Cloud Backup")
        }
    }
}

@Composable
private fun CloudBackupInfoCard(
    title: String,
    body: String,
) {
    Card(
        colors =
            CardDefaults.cardColors(
                containerColor = MaterialTheme.colorScheme.surfaceContainerLow,
            ),
    ) {
        Column(
            modifier = Modifier.padding(16.dp),
            verticalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            Text(title, fontWeight = FontWeight.SemiBold)
            Text(body, style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
        }
    }
}

@Composable
private fun CloudBackupChecklistRow(
    checked: Boolean,
    title: String,
    onCheckedChange: (Boolean) -> Unit,
) {
    val successColor = MaterialTheme.coveColors.systemGreen

    Row(
        modifier =
            Modifier
                .fillMaxWidth()
                .clickable { onCheckedChange(!checked) },
        verticalAlignment = Alignment.Top,
    ) {
        Icon(
            imageVector = if (checked) Icons.Default.CloudDone else Icons.Default.ErrorOutline,
            contentDescription = null,
            tint = if (checked) successColor else MaterialTheme.colorScheme.outline,
        )
        Spacer(modifier = Modifier.width(12.dp))
        Text(title, style = MaterialTheme.typography.bodyMedium)
    }
}
