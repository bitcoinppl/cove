@file:Suppress("PackageNaming")

package org.bitcoinppl.cove.flows.SettingsFlow

import android.content.ClipboardManager
import android.content.Context
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.test.platform.app.InstrumentationRegistry
import org.bitcoinppl.cove.ui.theme.CoveTheme
import org.junit.Assert.assertEquals
import org.junit.Rule
import org.junit.Test

class SendDiagnosticsCopyReportIdTest {
    @get:Rule
    val compose = createComposeRule()

    @Test
    fun copyIdPutsTheExactReportIdOnTheClipboard() {
        val reportId = "cvd_6da84a98ca2a4a72b25823938ab9c8f7"
        val context = InstrumentationRegistry.getInstrumentation().targetContext
        val clipboard = context.getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager
        compose.runOnUiThread { clipboard.setPrimaryClip(android.content.ClipData.newPlainText("seed", "stale")) }

        compose.setContent {
            val hostContext = LocalContext.current

            CoveTheme {
                SendDiagnosticsContent(
                    state =
                        SendDiagnosticsContentState(
                            description = "",
                            previewChunks = emptyList(),
                            reportSize = "1 KB",
                            feedback = DiagnosticsContentFeedback.Sent(reportId),
                            submitting = false,
                        ),
                    actions =
                        SendDiagnosticsContentActions(
                            onDescriptionChange = {},
                            onShare = {},
                            onClear = {},
                            onSubmit = {},
                            onSentReportAction = { id, action ->
                                if (action == SentReportAction.CopyReportId) copyReportId(hostContext, id)
                            },
                        ),
                )
            }
        }

        compose.onNodeWithText("Copy ID").performClick()

        var copied: String? = null
        compose.runOnUiThread {
            copied = clipboard.primaryClip?.getItemAt(0)?.text?.toString()
        }
        assertEquals(reportId, copied)
    }
}
