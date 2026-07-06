package org.bitcoinppl.cove.flows.SettingsFlow

import android.content.ClipData
import android.content.ClipboardManager
import android.content.Context
import android.content.Intent
import android.net.Uri
import android.os.Build
import android.widget.Toast
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.asPaddingValues
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.safeDrawing
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.FilledTonalButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.tooling.preview.Preview
import androidx.compose.ui.unit.dp
import androidx.core.content.FileProvider
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import org.bitcoinppl.cove.BuildConfig
import org.bitcoinppl.cove.ui.theme.CoveTheme
import org.bitcoinppl.cove_core.DiagnosticsPlatformInfo
import org.bitcoinppl.cove_core.DiagnosticsReport
import org.bitcoinppl.cove_core.buildDiagnosticsReport
import org.bitcoinppl.cove_core.clearDiagnosticsLogs
import java.io.File

private const val DIAGNOSTICS_FILENAME = "cove-diagnostics.txt"
private const val PREVIEW_CHUNK_SIZE = 4096
private const val MAX_PLATFORM_LOG_CHARS = 256 * 1024

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun SendDiagnosticsSheet(
    onDismiss: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val context = LocalContext.current
    val coroutineScope = rememberCoroutineScope()
    var report by remember { mutableStateOf<DiagnosticsReport?>(null) }
    val currentReport by rememberUpdatedState(report)
    var previewText by remember { mutableStateOf("") }
    var previewChunks by remember { mutableStateOf<List<String>>(emptyList()) }
    var description by remember { mutableStateOf("") }
    var reportSize by remember { mutableStateOf("") }
    var loadError by remember { mutableStateOf<String?>(null) }
    var actionError by remember { mutableStateOf<String?>(null) }
    var reportId by remember { mutableStateOf<String?>(null) }
    var showClearConfirmation by remember { mutableStateOf(false) }
    var loading by remember { mutableStateOf(true) }
    var submitting by remember { mutableStateOf(false) }

    fun replaceReport(nextReport: DiagnosticsReport?) {
        report?.close()
        report = nextReport
    }

    suspend fun rebuildReport(clearStoredLogs: Boolean) {
        loading = true
        loadError = null
        actionError = null
        reportId = null
        replaceReport(null)
        previewText = ""
        previewChunks = emptyList()
        reportSize = ""

        try {
            if (clearStoredLogs) {
                withContext(Dispatchers.IO) { clearDiagnosticsLogs() }
            }

            val platformLogs = collectAndroidPlatformLogs(context)
            val nextReport =
                buildDiagnosticsReport(
                    platform = androidDiagnosticsPlatformInfo(),
                    platformLogs = platformLogs,
                )
            val nextPreviewText = nextReport.previewText()

            replaceReport(nextReport)
            previewText = nextPreviewText
            previewChunks = nextPreviewText.chunked(PREVIEW_CHUNK_SIZE)
            reportSize = "${nextReport.sizeBytes()} bytes"
        } catch (error: Exception) {
            loadError = error.message ?: error.javaClass.simpleName
        } finally {
            loading = false
        }
    }

    LaunchedEffect(Unit) {
        rebuildReport(clearStoredLogs = false)
    }

    DisposableEffect(Unit) {
        onDispose {
            currentReport?.close()
        }
    }

    Scaffold(
        modifier =
            modifier
                .fillMaxSize()
                .padding(WindowInsets.safeDrawing.asPaddingValues()),
        topBar = {
            TopAppBar(
                title = {
                    Text(
                        text = "Send Diagnostics",
                        style = MaterialTheme.typography.bodyLarge,
                        textAlign = TextAlign.Center,
                        modifier = Modifier.fillMaxWidth(),
                    )
                },
                navigationIcon = {
                    androidx.compose.material3.IconButton(onClick = onDismiss) {
                        androidx.compose.material3.Icon(
                            Icons.AutoMirrored.Default.ArrowBack,
                            contentDescription = "Back",
                        )
                    }
                },
            )
        },
    ) { paddingValues ->
        when {
            loading -> {
                Column(
                    modifier =
                        Modifier
                            .fillMaxSize()
                            .padding(paddingValues),
                    verticalArrangement = Arrangement.Center,
                ) {
                    CircularProgressIndicator(
                        modifier = Modifier.padding(horizontal = 16.dp),
                    )
                    Text(
                        text = "Building diagnostics...",
                        style = MaterialTheme.typography.bodyMedium,
                        textAlign = TextAlign.Center,
                        modifier =
                            Modifier
                                .fillMaxWidth()
                                .padding(top = 12.dp),
                    )
                }
            }

            loadError != null -> {
                DiagnosticsLoadError(
                    message = loadError.orEmpty(),
                    onRetry = {
                        coroutineScope.launch {
                            rebuildReport(clearStoredLogs = false)
                        }
                    },
                    modifier =
                        Modifier
                            .fillMaxSize()
                            .padding(paddingValues),
                )
            }

            else -> {
                SendDiagnosticsContent(
                    description = description,
                    onDescriptionChange = { description = it },
                    previewChunks = previewChunks,
                    reportSize = reportSize,
                    actionError = actionError,
                    reportId = reportId,
                    submitting = submitting,
                    onShare = {
                        coroutineScope.launch {
                            runCatching {
                                shareDiagnosticsFile(
                                    context = context,
                                    content = exportText(previewText, description),
                                )
                            }.onFailure { error ->
                                actionError = error.message ?: error.javaClass.simpleName
                            }
                        }
                    },
                    onClear = { showClearConfirmation = true },
                    onSubmit = {
                        val current = report ?: return@SendDiagnosticsContent
                        coroutineScope.launch {
                            submitting = true
                            actionError = null

                            runCatching {
                                current.submit(trimmedDescription(description))
                            }.onSuccess { nextReportId ->
                                reportId = nextReportId
                            }.onFailure { error ->
                                actionError = error.message ?: error.javaClass.simpleName
                            }

                            submitting = false
                        }
                    },
                    onCopyReportId = { id ->
                        copyReportId(context, id)
                    },
                    onDone = onDismiss,
                    modifier =
                        Modifier
                            .fillMaxSize()
                            .padding(paddingValues),
                )
            }
        }
    }

    if (showClearConfirmation) {
        AlertDialog(
            onDismissRequest = { showClearConfirmation = false },
            title = { Text("Clear Stored Logs?") },
            text = { Text("This deletes stored diagnostics logs on this device and rebuilds the preview.") },
            confirmButton = {
                TextButton(
                    onClick = {
                        showClearConfirmation = false
                        coroutineScope.launch {
                            rebuildReport(clearStoredLogs = true)
                        }
                    },
                ) {
                    Text("Clear")
                }
            },
            dismissButton = {
                TextButton(onClick = { showClearConfirmation = false }) {
                    Text("Cancel")
                }
            },
        )
    }
}

@Composable
private fun DiagnosticsLoadError(
    message: String,
    onRetry: () -> Unit,
    modifier: Modifier = Modifier,
) {
    Column(
        modifier = modifier.padding(24.dp),
        verticalArrangement = Arrangement.Center,
    ) {
        Text(
            text = "Diagnostics Unavailable",
            style = MaterialTheme.typography.headlineSmall,
            textAlign = TextAlign.Center,
            modifier = Modifier.fillMaxWidth(),
        )
        Text(
            text = message,
            style = MaterialTheme.typography.bodyMedium,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            textAlign = TextAlign.Center,
            modifier =
                Modifier
                    .fillMaxWidth()
                    .padding(top = 8.dp),
        )
        Button(
            onClick = onRetry,
            modifier =
                Modifier
                    .fillMaxWidth()
                    .padding(top = 16.dp),
        ) {
            Text("Retry")
        }
    }
}

@Composable
internal fun SendDiagnosticsContent(
    description: String,
    onDescriptionChange: (String) -> Unit,
    previewChunks: List<String>,
    reportSize: String,
    actionError: String?,
    reportId: String?,
    submitting: Boolean,
    onShare: () -> Unit,
    onClear: () -> Unit,
    onSubmit: () -> Unit,
    onCopyReportId: (String) -> Unit,
    onDone: () -> Unit,
    modifier: Modifier = Modifier,
) {
    Column(
        modifier =
            modifier
                .padding(horizontal = 16.dp, vertical = 12.dp),
        verticalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        Text(
            text = "Description",
            style = MaterialTheme.typography.titleMedium,
        )

        OutlinedTextField(
            value = description,
            onValueChange = onDescriptionChange,
            placeholder = { Text("Optional") },
            minLines = 3,
            maxLines = 5,
            modifier = Modifier.fillMaxWidth(),
        )

        Row(
            modifier = Modifier.fillMaxWidth(),
            horizontalArrangement = Arrangement.SpaceBetween,
        ) {
            Text(
                text = "Preview",
                style = MaterialTheme.typography.titleMedium,
            )

            Text(
                text = reportSize,
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }

        LazyColumn(
            modifier =
                Modifier
                    .fillMaxWidth()
                    .weight(1f)
                    .clip(RoundedCornerShape(8.dp))
                    .background(MaterialTheme.colorScheme.surfaceVariant)
                    .padding(12.dp),
        ) {
            itemsIndexed(previewChunks) { _, chunk ->
                Text(
                    text = chunk,
                    style = MaterialTheme.typography.bodySmall,
                    fontFamily = FontFamily.Monospace,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
        }

        if (actionError != null) {
            Text(
                text = actionError,
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.error,
            )
        }

        if (reportId != null) {
            Column(
                modifier =
                    Modifier
                        .fillMaxWidth()
                        .clip(RoundedCornerShape(8.dp))
                        .background(MaterialTheme.colorScheme.surfaceVariant)
                        .padding(12.dp),
                verticalArrangement = Arrangement.spacedBy(8.dp),
            ) {
                Text(
                    text = "Diagnostics sent",
                    style = MaterialTheme.typography.titleMedium,
                )
                Text(
                    text = reportId,
                    style = MaterialTheme.typography.bodyMedium,
                    fontFamily = FontFamily.Monospace,
                )
                Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    FilledTonalButton(onClick = { onCopyReportId(reportId) }) {
                        Text("Copy ID")
                    }
                    FilledTonalButton(onClick = onDone) {
                        Text("Done")
                    }
                }
            }
        }

        Row(
            horizontalArrangement = Arrangement.spacedBy(8.dp),
            modifier = Modifier.fillMaxWidth(),
        ) {
            OutlinedButton(
                onClick = onShare,
                enabled = !submitting,
                modifier = Modifier.weight(1f),
            ) {
                Text("Share")
            }

            OutlinedButton(
                onClick = onClear,
                enabled = !submitting,
                colors =
                    ButtonDefaults.outlinedButtonColors(
                        contentColor = MaterialTheme.colorScheme.error,
                    ),
                modifier = Modifier.weight(1f),
            ) {
                Text(
                    text = "Clear Stored Logs",
                    textAlign = TextAlign.Center,
                )
            }
        }

        Button(
            onClick = onSubmit,
            enabled = !submitting && reportId == null,
            modifier =
                Modifier
                    .fillMaxWidth()
                    .heightIn(min = 48.dp),
        ) {
            if (submitting) {
                CircularProgressIndicator(
                    color = MaterialTheme.colorScheme.onPrimary,
                    strokeWidth = 2.dp,
                )
            } else {
                Text(
                    text = "Submit",
                    textAlign = TextAlign.Center,
                    modifier = Modifier.fillMaxWidth(),
                )
            }
        }
    }
}

private fun androidDiagnosticsPlatformInfo(): DiagnosticsPlatformInfo =
    DiagnosticsPlatformInfo(
        platform = "Android",
        buildNumber = BuildConfig.VERSION_CODE.toString(),
        osVersion = "Android ${Build.VERSION.RELEASE} (API ${Build.VERSION.SDK_INT})",
        deviceModel =
            listOf(Build.MANUFACTURER, Build.MODEL)
                .filter { it.isNotBlank() }
                .joinToString(" "),
    )

private suspend fun collectAndroidPlatformLogs(context: Context): String =
    withContext(Dispatchers.IO) {
        val header =
            listOf(
                "Generated: ${java.time.Instant.now()}",
                "App version: ${BuildConfig.VERSION_NAME}",
                "Build: ${BuildConfig.VERSION_CODE}",
                "Package: ${context.packageName}",
                "Android: ${Build.VERSION.RELEASE} (API ${Build.VERSION.SDK_INT})",
                "Device: ${Build.MANUFACTURER} ${Build.MODEL}",
                "Process ID: ${android.os.Process.myPid()}",
                "",
                "logcat",
            ).joinToString("\n")

        val logcat =
            runCatching {
                val process =
                    ProcessBuilder("logcat", "-d", "-t", "1000")
                        .redirectErrorStream(true)
                        .start()
                val output = process.inputStream.bufferedReader().use { it.readText() }
                val exitCode = process.waitFor()

                if (exitCode == 0) {
                    output.ifBlank { "logcat returned no visible app logs" }
                } else {
                    "logcat exited with code $exitCode\n$output"
                }
            }.getOrElse { error ->
                "logcat unavailable: ${error.message ?: error.javaClass.simpleName}"
            }

        "$header\n${logcat.takeLast(MAX_PLATFORM_LOG_CHARS)}"
    }

private fun trimmedDescription(description: String): String? {
    val trimmed = description.trim()
    return trimmed.ifEmpty { null }
}

private fun exportText(
    previewText: String,
    description: String,
): String {
    val trimmed = trimmedDescription(description) ?: return previewText

    return listOf(
        previewText,
        "",
        "User description",
        trimmed,
    ).joinToString("\n")
}

private suspend fun shareDiagnosticsFile(
    context: Context,
    content: String,
) {
    val uri: Uri =
        withContext(Dispatchers.IO) {
            val file = File(context.cacheDir, DIAGNOSTICS_FILENAME)
            file.writeText(content)

            FileProvider.getUriForFile(
                context,
                "${context.packageName}.fileprovider",
                file,
            )
        }

    val intent =
        Intent(Intent.ACTION_SEND).apply {
            type = "text/plain"
            putExtra(Intent.EXTRA_STREAM, uri)
            addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
        }

    context.startActivity(Intent.createChooser(intent, "Share Diagnostics"))
}

private fun copyReportId(
    context: Context,
    reportId: String,
) {
    val clipboard = context.getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager
    clipboard.setPrimaryClip(ClipData.newPlainText("Cove diagnostics report ID", reportId))

    if (Build.VERSION.SDK_INT < Build.VERSION_CODES.TIRAMISU) {
        Toast.makeText(context, "Report ID copied", Toast.LENGTH_SHORT).show()
    }
}

@Preview(
    name = "Send Diagnostics",
    showSystemUi = true,
    widthDp = 393,
    heightDp = 852,
)
@Composable
private fun SendDiagnosticsContentPreview() {
    CoveTheme(darkTheme = false, dynamicColor = false) {
        SendDiagnosticsContent(
            description = "Wallet could not sync.",
            onDescriptionChange = { },
            previewChunks =
                listOf(
                    """
                    App
                    Version: 1.3.0

                    Startup diagnostics
                    <redacted path>

                    Platform logs
                    logcat returned no visible app logs

                    Rust logs
                    [txid]
                    """.trimIndent(),
                ),
            reportSize = "12 KB",
            actionError = null,
            reportId = "diag_123456",
            submitting = false,
            onShare = { },
            onClear = { },
            onSubmit = { },
            onCopyReportId = { },
            onDone = { },
        )
    }
}
