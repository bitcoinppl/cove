@file:Suppress("FunctionNaming", "PackageNaming")

package org.bitcoinppl.cove.flows.SettingsFlow

import android.content.Context
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.asPaddingValues
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.safeDrawing
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
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
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.style.TextAlign
import kotlin.coroutines.cancellation.CancellationException
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.CoroutineDispatcher
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import org.bitcoinppl.cove_core.DiagnosticsReport
import org.bitcoinppl.cove_core.buildDiagnosticsReport
import org.bitcoinppl.cove_core.clearDiagnosticsLogs

private const val PREVIEW_CHUNK_SIZE = 4096
private const val PREVIEW_REFRESH_DEBOUNCE_MS = 250L

internal class DiagnosticsGenerationTracker {
    private var generation = 0

    fun advance(): Int {
        generation += 1

        return generation
    }

    fun invalidate() {
        generation += 1
    }

    fun isCurrent(token: Int): Boolean = token == generation
}

@Suppress("InjectDispatcher")
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun SendDiagnosticsSheet(
    onDismiss: () -> Unit,
    onSubmittingChange: (Boolean) -> Unit = { },
    modifier: Modifier = Modifier,
    ioDispatcher: CoroutineDispatcher = Dispatchers.IO,
) {
    val context = LocalContext.current
    val coroutineScope = rememberCoroutineScope()
    val state = remember(ioDispatcher, coroutineScope) {
        SendDiagnosticsSheetState(ioDispatcher, coroutineScope)
    }
    val currentOnSubmittingChange by rememberUpdatedState(onSubmittingChange)

    LaunchedEffect(Unit) {
        state.rebuildReport(context, clearStoredLogs = false)
    }

    LaunchedEffect(state.submitting) {
        onSubmittingChange(state.submitting)
    }

    DisposableEffect(Unit) {
        onDispose {
            currentOnSubmittingChange(false)
            state.close()
        }
    }

    SendDiagnosticsScaffold(
        state = state,
        actions =
            SendDiagnosticsSheetActions(
                onDismiss = onDismiss,
                onRetry = {
                    coroutineScope.launch {
                        state.rebuildReport(context, clearStoredLogs = false)
                    }
                },
                onShare = {
                    coroutineScope.launch {
                        state.share(context)
                    }
                },
                onClear = {
                    coroutineScope.launch {
                        state.rebuildReport(context, clearStoredLogs = true)
                    }
                },
                onSubmit = {
                    coroutineScope.launch {
                        state.submitCurrent()
                    }
                },
            ),
        modifier = modifier,
    )
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun SendDiagnosticsScaffold(
    state: SendDiagnosticsSheetState,
    actions: SendDiagnosticsSheetActions,
    modifier: Modifier = Modifier,
) {
    Scaffold(
        modifier =
            modifier
                .fillMaxSize()
                .padding(WindowInsets.safeDrawing.asPaddingValues()),
        topBar = {
            SendDiagnosticsTopBar(
                submitting = state.submitting,
                onDismiss = actions.onDismiss,
            )
        },
    ) { paddingValues ->
        SendDiagnosticsBody(
            state = state,
            actions = actions,
            paddingValues = paddingValues,
        )
    }

    ClearStoredLogsDialog(
        visible = state.showClearConfirmation,
        onDismiss = { state.showClearConfirmation = false },
        onConfirm = {
            state.showClearConfirmation = false
            actions.onClear()
        },
    )
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun SendDiagnosticsTopBar(
    submitting: Boolean,
    onDismiss: () -> Unit,
) {
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
            IconButton(
                onClick = {
                    if (!submitting) {
                        onDismiss()
                    }
                },
                enabled = !submitting,
            ) {
                Icon(
                    Icons.AutoMirrored.Default.ArrowBack,
                    contentDescription = "Back",
                )
            }
        },
    )
}

@Composable
private fun SendDiagnosticsBody(
    state: SendDiagnosticsSheetState,
    actions: SendDiagnosticsSheetActions,
    paddingValues: PaddingValues,
) {
    val context = LocalContext.current
    val modifier =
        Modifier
            .fillMaxSize()
            .padding(paddingValues)

    when {
        state.loading -> DiagnosticsLoading(modifier)
        state.loadError != null -> DiagnosticsLoadError(
            message = state.loadError.orEmpty(),
            onRetry = actions.onRetry,
            modifier = modifier,
        )
        else -> SendDiagnosticsContent(
            state = state.contentState(),
            actions =
                SendDiagnosticsContentActions(
                    onDescriptionChange = state::updateDescription,
                    onShare = actions.onShare,
                    onClear = { state.showClearConfirmation = true },
                    onSubmit = actions.onSubmit,
                    onSentReportAction = { reportId, action ->
                        when (action) {
                            SentReportAction.CopyReportId -> copyReportId(context, reportId)
                            SentReportAction.Done -> actions.onDismiss()
                        }
                    },
                ),
            modifier = modifier,
        )
    }
}

@Composable
private fun ClearStoredLogsDialog(
    visible: Boolean,
    onDismiss: () -> Unit,
    onConfirm: () -> Unit,
) {
    if (!visible) return

    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text("Clear Stored Logs?") },
        text = { Text("This deletes stored diagnostics logs on this device and rebuilds the preview.") },
        confirmButton = {
            TextButton(onClick = onConfirm) {
                Text("Clear")
            }
        },
        dismissButton = {
            TextButton(onClick = onDismiss) {
                Text("Cancel")
            }
        },
    )
}

private class SendDiagnosticsSheetState(
    private val ioDispatcher: CoroutineDispatcher,
    private val coroutineScope: CoroutineScope,
) {
    private var report by mutableStateOf<DiagnosticsReport?>(null)
    private val rebuildGeneration = DiagnosticsGenerationTracker()
    private var previewRefreshJob: Job? = null
    private var previewText by mutableStateOf("")
    var previewChunks by mutableStateOf<List<String>>(emptyList())
        private set
    var description by mutableStateOf("")
        private set
    var reportSize by mutableStateOf("")
        private set
    var loadError by mutableStateOf<String?>(null)
        private set
    var actionError by mutableStateOf<String?>(null)
        private set
    var reportId by mutableStateOf<String?>(null)
        private set
    private var submissionWarning by mutableStateOf<String?>(null)
    var showClearConfirmation by mutableStateOf(false)
    var loading by mutableStateOf(true)
        private set
    var submitting by mutableStateOf(false)
        private set

    suspend fun rebuildReport(
        context: Context,
        clearStoredLogs: Boolean,
    ) {
        val generation = rebuildGeneration.advance()
        var builtReport: DiagnosticsReport? = null
        startLoadingReport()

        try {
            if (clearStoredLogs) {
                withContext(ioDispatcher) { clearDiagnosticsLogs() }
            }

            val platformLogs = collectAndroidPlatformLogs(context, ioDispatcher)
            val nextReport =
                withContext(ioDispatcher) {
                    buildDiagnosticsReport(
                        platform = androidDiagnosticsPlatformInfo(),
                        platformLogs = platformLogs,
                    )
                }
            builtReport = nextReport
            val nextPreview = buildPreview(nextReport, description)

            if (!rebuildGeneration.isCurrent(generation)) {
                return
            }

            builtReport = null
            replaceReport(nextReport)
            applyPreview(nextPreview)
        } catch (error: CancellationException) {
            throw error
        } catch (error: Exception) {
            if (rebuildGeneration.isCurrent(generation)) {
                loadError = error.displayMessage()
            }
        } finally {
            builtReport?.close()
            if (rebuildGeneration.isCurrent(generation)) {
                loading = false
            }
        }
    }

    fun updateDescription(nextDescription: String) {
        description = nextDescription
        schedulePreviewRefresh()
    }

    suspend fun share(context: Context) {
        val content = report?.previewTextForDescription(description) ?: previewText
        try {
            shareDiagnosticsFile(context, content, ioDispatcher)
        } catch (error: CancellationException) {
            throw error
        } catch (error: Exception) {
            actionError = error.displayMessage()
        }
    }

    @Suppress("RedundantSuspendModifier")
    suspend fun submitCurrent() {
        val current = report ?: return
        submitting = true
        actionError = null

        try {
            val submission = withContext(ioDispatcher) { current.submit(description) }
            reportId = submission.reportId
            submissionWarning = submission.warning
        } catch (error: CancellationException) {
            throw error
        } catch (error: Exception) {
            actionError = error.displayMessage()
        } finally {
            submitting = false
        }
    }

    fun close() {
        rebuildGeneration.invalidate()
        replaceReport(null)
    }

    fun contentState(): SendDiagnosticsContentState =
        SendDiagnosticsContentState(
            description = description,
            previewChunks = previewChunks,
            reportSize = reportSize,
            feedback = feedback(),
            submitting = submitting,
        )

    private fun startLoadingReport() {
        loading = true
        loadError = null
        actionError = null
        reportId = null
        submissionWarning = null
        replaceReport(null)
        previewText = ""
        previewChunks = emptyList()
        reportSize = ""
    }

    private fun replaceReport(nextReport: DiagnosticsReport?) {
        val oldReport = report
        val oldPreviewRefreshJob = previewRefreshJob
        oldPreviewRefreshJob?.cancel()
        previewRefreshJob = null
        report = nextReport
        closeReportAfterPreview(oldReport, oldPreviewRefreshJob)
    }

    private fun closeReportAfterPreview(
        oldReport: DiagnosticsReport?,
        oldPreviewRefreshJob: Job?,
    ) {
        if (oldReport == null) return

        if (oldPreviewRefreshJob == null || oldPreviewRefreshJob.isCompleted) {
            oldReport.close()
            return
        }

        oldPreviewRefreshJob.invokeOnCompletion {
            oldReport.close()
        }
    }

    private fun schedulePreviewRefresh() {
        val currentReport = report ?: return
        val currentDescription = description
        previewRefreshJob?.cancel()
        previewRefreshJob =
            coroutineScope.launch {
                try {
                    delay(PREVIEW_REFRESH_DEBOUNCE_MS)
                    val nextPreview = buildPreview(currentReport, currentDescription)

                    if (report == currentReport && description == currentDescription) {
                        applyPreview(nextPreview)
                    }
                } finally {
                    if (previewRefreshJob == this.coroutineContext[Job]) {
                        previewRefreshJob = null
                    }
                }
            }
    }

    private suspend fun buildPreview(
        nextReport: DiagnosticsReport,
        nextDescription: String,
    ): DiagnosticsPreviewState =
        withContext(ioDispatcher) {
            val nextPreviewText = nextReport.previewTextForDescription(nextDescription)

            DiagnosticsPreviewState(
                text = nextPreviewText,
                chunks = nextPreviewText.chunked(PREVIEW_CHUNK_SIZE),
                formattedSize = nextReport.formattedSizeForDescription(nextDescription),
            )
        }

    private fun applyPreview(nextPreview: DiagnosticsPreviewState) {
        previewText = nextPreview.text
        previewChunks = nextPreview.chunks
        reportSize = nextPreview.formattedSize
    }

    private fun feedback(): DiagnosticsContentFeedback =
        actionError?.let { DiagnosticsContentFeedback.Error(it) }
            ?: reportId?.let { DiagnosticsContentFeedback.Sent(it, submissionWarning) }
            ?: DiagnosticsContentFeedback.None
}

private data class DiagnosticsPreviewState(
    val text: String,
    val chunks: List<String>,
    val formattedSize: String,
)

private data class SendDiagnosticsSheetActions(
    val onDismiss: () -> Unit,
    val onRetry: () -> Unit,
    val onShare: () -> Unit,
    val onClear: () -> Unit,
    val onSubmit: () -> Unit,
)
