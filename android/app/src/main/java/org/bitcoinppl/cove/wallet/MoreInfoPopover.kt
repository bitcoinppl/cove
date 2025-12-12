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
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import org.bitcoinppl.cove.AppAlertState
import org.bitcoinppl.cove.AppManager
import org.bitcoinppl.cove.TaggedItem
import org.bitcoinppl.cove.WalletManager

// delay before showing export loading alert
private const val EXPORT_LOADING_ALERT_DELAY_MS = 500L

// delay before showing file picker after dismissing alert
private const val ALERT_DISMISS_DELAY_MS = 500L

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
                    exportState.isImporting = true
                    try {
                        val fileContents =
                            withContext(Dispatchers.IO) {
                                context.contentResolver.openInputStream(uri)?.use { input ->
                                    input.bufferedReader().use { it.readText() }
                                }
                            } ?: throw Exception("Unable to read file")

                        // validate import was successful before showing success message
                        (
                            manager?.rust?.labelManager()
                                ?: throw Exception("Label manager not available")
                        ).use { labelManager ->
                            labelManager.import(fileContents.trim())
                        }

                        // refresh transactions with updated labels
                        try {
                            manager.rust.getTransactions()
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
                    exportState.isExporting = true
                    val currentExportType = exportState.exportType
                    var showedLoadingAlert = false

                    try {
                        // show loading alert for transaction exports after a delay
                        val alertJob =
                            scope.launch {
                                delay(EXPORT_LOADING_ALERT_DELAY_MS)
                                if (exportState.isExporting && currentExportType is ExportType.Transactions) {
                                    app.alertState =
                                        TaggedItem(
                                            AppAlertState.General(
                                                title = "Exporting, please wait...",
                                                message = "Creating a transaction export file. If this is the first time it might take a while",
                                            ),
                                        )
                                    showedLoadingAlert = true
                                }
                            }

                        val content =
                            when (currentExportType) {
                                is ExportType.Transactions -> {
                                    withContext(Dispatchers.IO) {
                                        manager?.rust?.createTransactionsWithFiatExport()
                                    }
                                }
                                is ExportType.Labels -> {
                                    withContext(Dispatchers.IO) {
                                        manager?.rust?.labelManager()?.use { it.export() }
                                    }
                                }
                                null -> null
                            }

                        // cancel and clear loading alert if shown
                        alertJob.cancel()
                        if (showedLoadingAlert) {
                            app.alertState = null
                            delay(ALERT_DISMISS_DELAY_MS)
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
                        // clear any loading alert on error
                        if (showedLoadingAlert) {
                            app.alertState = null
                        }

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
