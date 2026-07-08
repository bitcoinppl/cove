@file:Suppress("FunctionNaming", "PackageNaming")

package org.bitcoinppl.cove.flows.SettingsFlow

import android.content.Context
import android.util.Log
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.asPaddingValues
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.safeDrawing
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.filled.ContentCopy
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.ListItem
import androidx.compose.material3.ListItemDefaults
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import java.time.Instant
import java.time.ZoneId
import java.time.format.DateTimeFormatter
import java.time.format.FormatStyle
import org.bitcoinppl.cove.views.MaterialDivider
import org.bitcoinppl.cove_core.Database
import org.bitcoinppl.cove_core.DiagnosticsReportRecord

private const val TAG = "SubmittedDiagnostics"

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun SubmittedDiagnosticsScreen(
    onDismiss: () -> Unit,
    onRecordsChanged: () -> Unit,
    modifier: Modifier = Modifier,
) {
    var records by remember { mutableStateOf<List<DiagnosticsReportRecord>>(emptyList()) }
    var showClearConfirmation by remember { mutableStateOf(false) }
    var actionError by remember { mutableStateOf<String?>(null) }

    LaunchedEffect(Unit) {
        records = loadSubmittedDiagnosticsRecords()
    }

    Scaffold(
        modifier =
            modifier
                .fillMaxSize()
                .padding(WindowInsets.safeDrawing.asPaddingValues()),
        topBar = {
            SubmittedDiagnosticsTopBar(
                hasRecords = records.isNotEmpty(),
                onDismiss = onDismiss,
                onClear = { showClearConfirmation = true },
            )
        },
    ) { paddingValues ->
        SubmittedDiagnosticsBody(
            records = records,
            paddingValues = paddingValues,
        )
    }

    ClearSubmittedDiagnosticsDialog(
        visible = showClearConfirmation,
        onDismiss = { showClearConfirmation = false },
        onConfirm = {
            showClearConfirmation = false
            try {
                Database().diagnosticsReports().clear()
                records = emptyList()
                onRecordsChanged()
            } catch (error: Exception) {
                actionError = error.displayMessage()
            }
        },
    )

    actionError?.let { error ->
        AlertDialog(
            onDismissRequest = { actionError = null },
            title = { Text("Something went wrong") },
            text = { Text(error) },
            confirmButton = {
                TextButton(onClick = { actionError = null }) {
                    Text("OK")
                }
            },
        )
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun SubmittedDiagnosticsTopBar(
    hasRecords: Boolean,
    onDismiss: () -> Unit,
    onClear: () -> Unit,
) {
    TopAppBar(
        title = {
            Text(
                text = "Submitted Diagnostics",
                style = MaterialTheme.typography.bodyLarge,
                textAlign = TextAlign.Center,
                modifier = Modifier.fillMaxWidth(),
            )
        },
        navigationIcon = {
            IconButton(onClick = onDismiss) {
                Icon(Icons.AutoMirrored.Default.ArrowBack, contentDescription = "Back")
            }
        },
        actions = {
            TextButton(
                onClick = onClear,
                enabled = hasRecords,
            ) {
                Text(
                    text = "Clear",
                    color =
                        if (hasRecords) {
                            MaterialTheme.colorScheme.error
                        } else {
                            MaterialTheme.colorScheme.onSurface.copy(alpha = 0.38f)
                        },
                )
            }
        },
    )
}

@Composable
private fun SubmittedDiagnosticsBody(
    records: List<DiagnosticsReportRecord>,
    paddingValues: PaddingValues,
) {
    if (records.isEmpty()) {
        Box(
            modifier =
                Modifier
                    .fillMaxSize()
                    .padding(paddingValues),
            contentAlignment = Alignment.Center,
        ) {
            Column(horizontalAlignment = Alignment.CenterHorizontally) {
                Text(
                    text = "No submitted diagnostics",
                    style = MaterialTheme.typography.titleMedium,
                )
                Text(
                    text = "Submitted report IDs will appear here.",
                    style = MaterialTheme.typography.bodyMedium,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
        }
        return
    }

    val context = LocalContext.current

    LazyColumn(
        modifier =
            Modifier
                .fillMaxSize()
                .padding(paddingValues),
    ) {
        itemsIndexed(
            items = records,
            key = { index, record -> "${record.reportId}-$index" },
        ) { index, record ->
            SubmittedDiagnosticsRow(
                context = context,
                record = record,
            )

            if (index < records.lastIndex) {
                MaterialDivider()
            }
        }
    }
}

@Composable
private fun SubmittedDiagnosticsRow(
    context: Context,
    record: DiagnosticsReportRecord,
) {
    ListItem(
        headlineContent = {
            Text(
                text = record.reportId,
                style = MaterialTheme.typography.bodyMedium,
                fontFamily = FontFamily.Monospace,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
            )
        },
        supportingContent = {
            Column {
                Text(
                    text = formattedSubmittedAt(record.submittedAt),
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )

                record.description?.takeIf { it.isNotBlank() }?.let { description ->
                    Text(
                        text = description,
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                        maxLines = 2,
                        overflow = TextOverflow.Ellipsis,
                    )
                }
            }
        },
        trailingContent = {
            IconButton(onClick = { copyReportId(context, record.reportId) }) {
                Icon(
                    Icons.Default.ContentCopy,
                    contentDescription = "Copy Report ID",
                    tint = MaterialTheme.colorScheme.primary,
                )
            }
        },
        colors =
            ListItemDefaults.colors(
                containerColor = MaterialTheme.colorScheme.background,
            ),
    )
}

@Composable
private fun ClearSubmittedDiagnosticsDialog(
    visible: Boolean,
    onDismiss: () -> Unit,
    onConfirm: () -> Unit,
) {
    if (!visible) return

    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text("Clear Submitted Diagnostics?") },
        text = { Text("This removes saved report IDs from this device.") },
        confirmButton = {
            TextButton(onClick = onConfirm) {
                Text(
                    text = "Clear",
                    color = MaterialTheme.colorScheme.error,
                )
            }
        },
        dismissButton = {
            TextButton(onClick = onDismiss) {
                Text("Cancel")
            }
        },
    )
}

internal fun loadSubmittedDiagnosticsRecords(): List<DiagnosticsReportRecord> =
    try {
        Database().diagnosticsReports().all()
    } catch (error: Exception) {
        Log.w(TAG, "Failed to load submitted diagnostics", error)
        emptyList()
    }

private fun formattedSubmittedAt(timestamp: ULong): String =
    DateTimeFormatter
        .ofLocalizedDateTime(FormatStyle.MEDIUM, FormatStyle.SHORT)
        .withZone(ZoneId.systemDefault())
        .format(Instant.ofEpochSecond(timestamp.toLong()))
