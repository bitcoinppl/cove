package org.bitcoinppl.cove.views

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.ContentCopy
import androidx.compose.material.icons.filled.Download
import androidx.compose.material.icons.filled.QrCodeScanner
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Icon
import androidx.compose.material3.ListItem
import androidx.compose.material3.ListItemDefaults
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.tooling.preview.Preview
import androidx.compose.ui.unit.dp
import org.bitcoinppl.cove.ui.theme.CoveTheme

internal data class DialogChoice(
    val label: String,
    val supportingText: String? = null,
    val icon: ImageVector? = null,
    val emphasized: Boolean = false,
    val onClick: () -> Unit,
)

@Composable
internal fun ChoiceAlertDialog(
    title: String,
    message: String? = null,
    choices: List<DialogChoice>,
    onDismiss: () -> Unit,
    onCancel: () -> Unit = onDismiss,
    cancelText: String = "Cancel",
    showCancelButton: Boolean = true,
) {
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text(title) },
        text = {
            Column(modifier = Modifier.fillMaxWidth()) {
                message?.let {
                    Text(
                        text = it,
                        style = MaterialTheme.typography.bodyMedium,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                    Spacer(modifier = Modifier.height(12.dp))
                }

                choices.forEach { choice ->
                    DialogChoiceRow(choice)
                }
            }
        },
        confirmButton = {
            if (showCancelButton) {
                TextButton(onClick = onCancel) {
                    Text(cancelText)
                }
            }
        },
    )
}

@Composable
private fun DialogChoiceRow(choice: DialogChoice) {
    val contentColor =
        if (choice.emphasized) {
            MaterialTheme.colorScheme.onPrimaryContainer
        } else {
            MaterialTheme.colorScheme.onSurface
        }
    val containerColor =
        if (choice.emphasized) {
            MaterialTheme.colorScheme.primaryContainer
        } else {
            Color.Transparent
        }

    ListItem(
        headlineContent = {
            Text(
                text = choice.label,
                color = contentColor,
            )
        },
        supportingContent =
            choice.supportingText?.let { supportingText ->
                {
                    Text(
                        text = supportingText,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
            },
        leadingContent =
            choice.icon?.let { icon ->
                {
                    Icon(
                        imageVector = icon,
                        contentDescription = null,
                        tint = contentColor,
                    )
                }
            },
        modifier =
            Modifier
                .fillMaxWidth()
                .clip(RoundedCornerShape(12.dp))
                .clickable(
                    role = Role.Button,
                    onClick = choice.onClick,
                ),
        colors =
            ListItemDefaults.colors(
                containerColor = containerColor,
            ),
    )
}

@Preview(showSystemUi = true, widthDp = 393, heightDp = 852)
@Composable
private fun ChoiceAlertDialogPreview() {
    ChoiceAlertDialogPreviewContent()
}

@Composable
internal fun ChoiceAlertDialogPreviewContent() {
    CoveTheme(darkTheme = false, dynamicColor = false) {
        Box(
            modifier =
                Modifier
                    .fillMaxSize()
                    .background(MaterialTheme.colorScheme.background),
        ) {
            ChoiceAlertDialog(
                title = "Choose import method",
                message = "Select how you want to restore this wallet.",
                choices =
                    listOf(
                        DialogChoice(
                            label = "Scan QR",
                            supportingText = "Use a wallet export code",
                            icon = Icons.Default.QrCodeScanner,
                            onClick = {},
                        ),
                        DialogChoice(
                            label = "Paste from clipboard",
                            supportingText = "Use copied wallet data",
                            icon = Icons.Default.ContentCopy,
                            onClick = {},
                        ),
                        DialogChoice(
                            label = "Download backup",
                            supportingText = "Restore from Cloud Backup",
                            icon = Icons.Default.Download,
                            emphasized = true,
                            onClick = {},
                        ),
                    ),
                onDismiss = {},
            )
        }
    }
}

@Preview(showSystemUi = true, widthDp = 393, heightDp = 852)
@Composable
private fun ChoiceAlertDialogNoCancelPreview() {
    ChoiceAlertDialogNoCancelPreviewContent()
}

@Composable
internal fun ChoiceAlertDialogNoCancelPreviewContent() {
    CoveTheme(darkTheme = false, dynamicColor = false) {
        Box(
            modifier =
                Modifier
                    .fillMaxSize()
                    .background(MaterialTheme.colorScheme.background),
        ) {
            ChoiceAlertDialog(
                title = "Found Address",
                message = "Choose where to send this payment.",
                choices =
                    listOf(
                        DialogChoice(
                            label = "Send To Address",
                            supportingText = "Use the address from the scanned request",
                            icon = Icons.Default.QrCodeScanner,
                            emphasized = true,
                            onClick = {},
                        ),
                        DialogChoice(
                            label = "Copy Address",
                            supportingText = "Review the address before sending",
                            icon = Icons.Default.ContentCopy,
                            onClick = {},
                        ),
                    ),
                onDismiss = {},
                showCancelButton = false,
            )
        }
    }
}
