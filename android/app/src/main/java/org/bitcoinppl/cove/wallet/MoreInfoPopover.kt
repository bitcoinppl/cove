package org.bitcoinppl.cove.wallet

import androidx.activity.compose.rememberLauncherForActivityResult
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

// export type for tracking what is being exported
sealed class ExportType {
    data object Labels : ExportType()

    data object Transactions : ExportType()
}

class WalletExportState {
    var exportType by mutableStateOf<ExportType?>(null)
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
        rememberLauncherForActivityResult(ActivityResultContracts.OpenDocument()) { uri ->
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
                        currentManager.rust.labelManager().use { labelManager ->
                            labelManager.import(fileContents.trim())
                        }

                        // refresh transactions with updated labels
                        try {
                            currentManager.rust.getTransactions()
                        } catch (refreshError: Exception) {
                            android.util.Log.e(tag, "failed to refresh transactions after label import", refreshError)
                            snackbarHostState.showSnackbar("Labels imported successfully, but transaction list may need manual refresh")
                            return@launch
                        }

                        snackbarHostState.showSnackbar("Labels imported successfully")
                    } catch (e: Exception) {
                        android.util.Log.e(tag, "error importing labels", e)
                        snackbarHostState.showSnackbar("Unable to import labels: ${e.localizedMessage ?: e.message}")
                    } finally {
                        exportState.isImporting = false
                    }
                }
            }
        }

    // file export launcher (for labels and transactions)
    val exportFileLauncher =
        rememberLauncherForActivityResult(
            ActivityResultContracts.CreateDocument("text/plain"),
        ) { uri ->
            uri?.let {
                scope.launch {
                    val currentManager = manager
                    exportState.isExporting = true
                    val currentExportType = exportState.exportType

                    try {
                        // fetch content using new async methods that handle loading popup
                        val content =
                            when (currentExportType) {
                                is ExportType.Transactions -> {
                                    currentManager?.rust?.exportTransactionsCsv()?.content
                                }
                                is ExportType.Labels -> {
                                    currentManager?.rust?.exportLabelsForShare()?.content
                                }
                                null -> null
                            }

                        content?.let { data ->
                            withContext(Dispatchers.IO) {
                                context.contentResolver.openOutputStream(uri)?.use { output ->
                                    output.bufferedWriter().use { it.write(data) }
                                }
                            }

                            val message =
                                when (currentExportType) {
                                    is ExportType.Transactions -> "Transactions exported successfully"
                                    is ExportType.Labels -> "Labels exported successfully"
                                    null -> "Export completed"
                                }
                            snackbarHostState.showSnackbar(message)
                        } ?: run {
                            val errorType =
                                when (currentExportType) {
                                    is ExportType.Transactions -> "transactions"
                                    is ExportType.Labels -> "labels"
                                    null -> "export"
                                }
                            snackbarHostState.showSnackbar("Unable to generate $errorType export data")
                        }
                    } catch (e: Exception) {
                        android.util.Log.e(tag, "error exporting file", e)
                        val errorType =
                            when (currentExportType) {
                                is ExportType.Transactions -> "transactions"
                                is ExportType.Labels -> "labels"
                                null -> "export"
                            }
                        snackbarHostState.showSnackbar("Unable to export $errorType: ${e.localizedMessage ?: e.message}")
                    } finally {
                        exportState.isExporting = false
                        exportState.exportType = null
                    }
                }
            } ?: run {
                // reset state if user cancelled the document picker
                exportState.exportType = null
            }
        }

    return WalletExportLaunchers(
        importLabels = { importLabelLauncher.launch(arrayOf("text/plain", "application/json", "application/x-jsonlines")) },
        exportLabels = { fileName ->
            exportState.exportType = ExportType.Labels
            exportFileLauncher.launch(fileName)
        },
        exportTransactions = { fileName ->
            exportState.exportType = ExportType.Transactions
            exportFileLauncher.launch(fileName)
        },
    )
}

data class WalletExportLaunchers(
    val importLabels: () -> Unit,
    val exportLabels: (String) -> Unit,
    val exportTransactions: (String) -> Unit,
)
