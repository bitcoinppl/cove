package org.bitcoinppl.cove.wallet

import android.net.Uri
import androidx.activity.compose.ManagedActivityResultLauncher
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.material3.SnackbarHostState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.platform.LocalContext
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import org.bitcoinppl.cove.AppManager
import org.bitcoinppl.cove.WalletManager
import org.bitcoinppl.cove.views.rememberLockAwareLauncherForActivityResult
import org.bitcoinppl.cove.Log

// what a wallet file export writes
sealed class ExportType {
    data object Labels : ExportType()

    data object Transactions : ExportType()
}

class WalletExportState {
    var isExporting by mutableStateOf(false)
    var isImporting by mutableStateOf(false)
}

@Composable
fun rememberWalletExportLaunchers(
    app: AppManager,
    manager: WalletManager?,
    snackbarHostState: SnackbarHostState,
    exportState: WalletExportState,
    tag: String = "WalletExportManager",
): WalletExportLaunchers {
    val context = LocalContext.current
    val scope = rememberCoroutineScope()

    // file import launcher (for labels) - accepts plain text and JSON files
    val importLabelLauncher =
        rememberLockAwareLauncherForActivityResult(ActivityResultContracts.OpenDocument()) { uri ->
            uri?.let {
                scope.launch {
                    // capture manager at coroutine start to avoid null during suspension
                    val currentManager =
                        manager ?: run {
                            snackbarHostState.showSnackbar("Manager not available")
                            return@launch
                        }

                    exportState.isImporting = true
                    try {
                        val fileContents =
                            withContext(Dispatchers.IO) {
                                context.contentResolver.openInputStream(uri)?.use { input ->
                                    input.bufferedReader().use { it.readText() }
                                }
                            } ?: throw Exception("Unable to read file")

                        // validate import was successful before showing success message
                        currentManager.labelManager().use { labelManager ->
                            labelManager.import(fileContents.trim())
                        }

                        val refreshed =
                            AppManager.getInstance()
                                .reconcileAfterLabelImportAndWait(currentManager.id)
                        if (!refreshed) {
                            snackbarHostState.showSnackbar("Labels imported successfully, but transaction list may need manual refresh")
                            return@launch
                        }

                        snackbarHostState.showSnackbar("Labels imported successfully")
                    } catch (e: Exception) {
                        Log.e(tag, "error importing labels", e)
                        snackbarHostState.showSnackbar("Unable to import labels: ${e.localizedMessage ?: e.message}")
                    } finally {
                        exportState.isImporting = false
                    }
                }
            }
        }

    // one launcher per export kind, so a destination held until unlock still knows what to write
    // after the activity is recreated
    val exportLabelsLauncher =
        rememberExportFileLauncher(ExportType.Labels, manager, snackbarHostState, exportState, tag)
    val exportTransactionsLauncher =
        rememberExportFileLauncher(ExportType.Transactions, manager, snackbarHostState, exportState, tag)

    return WalletExportLaunchers(
        importLabels = { importLabelLauncher.launch(arrayOf("text/plain", "application/json", "application/x-jsonlines")) },
        exportLabels = { fileName -> exportLabelsLauncher.launch(fileName) },
        exportTransactions = { fileName -> exportTransactionsLauncher.launch(fileName) },
    )
}

@Composable
private fun rememberExportFileLauncher(
    exportType: ExportType,
    manager: WalletManager?,
    snackbarHostState: SnackbarHostState,
    exportState: WalletExportState,
    tag: String,
): ManagedActivityResultLauncher<String, Uri?> {
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    val (exportName, successMessage) =
        when (exportType) {
            is ExportType.Transactions -> "transactions" to "Transactions exported successfully"
            is ExportType.Labels -> "labels" to "Labels exported successfully"
        }

    return rememberLockAwareLauncherForActivityResult(
        ActivityResultContracts.CreateDocument("text/plain"),
    ) { uri ->
        if (uri == null) return@rememberLockAwareLauncherForActivityResult

        scope.launch {
            exportState.isExporting = true

            try {
                // fetch content using new async methods that handle loading popup
                val content =
                    when (exportType) {
                        is ExportType.Transactions -> manager?.exportTransactionsCsv()?.content
                        is ExportType.Labels -> manager?.exportLabelsForShare()?.content
                    }

                if (content == null) {
                    snackbarHostState.showSnackbar("Unable to generate $exportName export data")
                    return@launch
                }

                withContext(Dispatchers.IO) {
                    context.contentResolver.openOutputStream(uri)?.use { output ->
                        output.bufferedWriter().use { it.write(content) }
                    }
                }

                snackbarHostState.showSnackbar(successMessage)
            } catch (e: Exception) {
                Log.e(tag, "error exporting file", e)
                snackbarHostState.showSnackbar("Unable to export $exportName: ${e.localizedMessage ?: e.message}")
            } finally {
                exportState.isExporting = false
            }
        }
    }
}

data class WalletExportLaunchers(
    val importLabels: () -> Unit,
    val exportLabels: (String) -> Unit,
    val exportTransactions: (String) -> Unit,
)
