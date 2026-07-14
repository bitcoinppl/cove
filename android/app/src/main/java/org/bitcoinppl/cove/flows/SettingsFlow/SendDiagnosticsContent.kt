@file:Suppress("FunctionNaming", "PackageNaming")

package org.bitcoinppl.cove.flows.SettingsFlow

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.FilledTonalButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.tooling.preview.Preview
import androidx.compose.ui.unit.dp
import org.bitcoinppl.cove.ui.theme.CoveTheme

@Composable
internal fun DiagnosticsLoading(modifier: Modifier = Modifier) {
    Column(
        modifier = modifier,
        horizontalAlignment = Alignment.CenterHorizontally,
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

@Composable
internal fun DiagnosticsLoadError(
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
    state: SendDiagnosticsContentState,
    actions: SendDiagnosticsContentActions,
    modifier: Modifier = Modifier,
) {
    Column(
        modifier = modifier.padding(horizontal = 16.dp, vertical = 12.dp),
        verticalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        DiagnosticsDescriptionField(state, actions)
        DiagnosticsPreview(state)
        DiagnosticsFeedback(state.feedback, actions)
        DiagnosticsActionButtons(state, actions)
    }
}

@Composable
private fun DiagnosticsDescriptionField(
    state: SendDiagnosticsContentState,
    actions: SendDiagnosticsContentActions,
) {
    Text(
        text = "Description",
        style = MaterialTheme.typography.titleMedium,
    )

    OutlinedTextField(
        value = state.description,
        onValueChange = actions.onDescriptionChange,
        placeholder = { Text("Optional") },
        minLines = 3,
        maxLines = 5,
        modifier = Modifier.fillMaxWidth(),
    )
}

@Composable
private fun ColumnScope.DiagnosticsPreview(state: SendDiagnosticsContentState) {
    Row(
        modifier = Modifier.fillMaxWidth(),
        horizontalArrangement = Arrangement.SpaceBetween,
    ) {
        Text(
            text = "Preview",
            style = MaterialTheme.typography.titleMedium,
        )

        Text(
            text = state.reportSize,
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
        items(state.previewChunks) { chunk ->
            Text(
                text = chunk,
                style = MaterialTheme.typography.bodySmall,
                fontFamily = FontFamily.Monospace,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
    }
}

@Composable
private fun DiagnosticsFeedback(
    feedback: DiagnosticsContentFeedback,
    actions: SendDiagnosticsContentActions,
) {
    when (feedback) {
        DiagnosticsContentFeedback.None -> Unit
        is DiagnosticsContentFeedback.Error -> DiagnosticsActionError(feedback.message)
        is DiagnosticsContentFeedback.Sent -> {
            DiagnosticsSentReport(feedback.reportId, feedback.warning, actions)
        }
    }
}

@Composable
private fun DiagnosticsActionError(message: String) {
    Text(
        text = message,
        style = MaterialTheme.typography.bodySmall,
        color = MaterialTheme.colorScheme.error,
    )
}

@Composable
private fun DiagnosticsSentReport(
    reportId: String,
    warning: String?,
    actions: SendDiagnosticsContentActions,
) {
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
        warning?.let { message ->
            Text(
                text = message,
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.error,
            )
        }
        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            FilledTonalButton(
                onClick = {
                    actions.onSentReportAction(reportId, SentReportAction.CopyReportId)
                },
            ) {
                Text("Copy ID")
            }
            FilledTonalButton(
                onClick = {
                    actions.onSentReportAction(reportId, SentReportAction.Done)
                },
            ) {
                Text("Done")
            }
        }
    }
}

@Composable
private fun DiagnosticsActionButtons(
    state: SendDiagnosticsContentState,
    actions: SendDiagnosticsContentActions,
) {
    Row(
        horizontalArrangement = Arrangement.spacedBy(8.dp),
        modifier = Modifier.fillMaxWidth(),
    ) {
        OutlinedButton(
            onClick = actions.onShare,
            enabled = !state.submitting,
            modifier = Modifier.weight(1f),
        ) {
            Text("Share")
        }

        OutlinedButton(
            onClick = actions.onClear,
            enabled = !state.submitting,
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
        onClick = actions.onSubmit,
        enabled = !state.submitting && state.feedback !is DiagnosticsContentFeedback.Sent,
        modifier =
            Modifier
                .fillMaxWidth()
                .heightIn(min = 48.dp),
    ) {
        if (state.submitting) {
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

@Preview(
    name = "Send Diagnostics",
    showSystemUi = true,
    widthDp = 393,
    heightDp = 852,
)
@Composable
internal fun SendDiagnosticsContentPreview() {
    SendDiagnosticsContentPreviewContent()
}

@Composable
internal fun SendDiagnosticsContentPreviewContent() {
    CoveTheme(darkTheme = false, dynamicColor = false) {
        SendDiagnosticsContent(
            state =
                SendDiagnosticsContentState(
                    description = "Wallet could not sync.",
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
                    feedback = DiagnosticsContentFeedback.Sent("diag_123456"),
                    submitting = false,
                ),
            actions =
                SendDiagnosticsContentActions(
                    onDescriptionChange = { },
                    onShare = { },
                    onClear = { },
                    onSubmit = { },
                    onSentReportAction = { _, _ -> },
                ),
        )
    }
}

internal data class SendDiagnosticsContentState(
    val description: String,
    val previewChunks: List<String>,
    val reportSize: String,
    val feedback: DiagnosticsContentFeedback,
    val submitting: Boolean,
)

internal sealed interface DiagnosticsContentFeedback {
    data object None : DiagnosticsContentFeedback

    data class Error(val message: String) : DiagnosticsContentFeedback

    data class Sent(
        val reportId: String,
        val warning: String? = null,
    ) : DiagnosticsContentFeedback
}

internal data class SendDiagnosticsContentActions(
    val onDescriptionChange: (String) -> Unit,
    val onShare: () -> Unit,
    val onClear: () -> Unit,
    val onSubmit: () -> Unit,
    val onSentReportAction: (String, SentReportAction) -> Unit,
)

internal enum class SentReportAction {
    CopyReportId,
    Done,
}
