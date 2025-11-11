package org.bitcoinppl.cove

import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.material3.SnackbarHostState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import org.bitcoinppl.cove.components.FullPageLoadingView
import org.bitcoinppl.cove.wallet_transactions.WalletMoreOptionsSheet
import org.bitcoinppl.cove.wallet_transactions.WalletTransactionsScreen
import org.bitcoinppl.cove_core.*
import org.bitcoinppl.cove_core.types.*

// delay to allow UI to settle before updating balance
private const val BALANCE_UPDATE_DELAY_MS = 500L

// delay before starting wallet scan to allow initial load to complete
private const val WALLET_SCAN_DELAY_MS = 400L

// delay before showing export loading alert
private const val EXPORT_LOADING_ALERT_DELAY_MS = 500L

// delay before showing file picker after dismissing alert
private const val ALERT_DISMISS_DELAY_MS = 500L

// export type for tracking what is being exported
sealed class ExportType {
    data object Labels : ExportType()

    data object Transactions : ExportType()
}

/**
 * selected wallet container - manages WalletManager lifecycle
 * ported from iOS SelectedWalletContainer.swift
 */
@Composable
fun SelectedWalletContainer(
    app: AppManager,
    id: WalletId,
    modifier: Modifier = Modifier,
) {
    var manager by remember { mutableStateOf<WalletManager?>(null) }
    var loadedId by remember { mutableStateOf<WalletId?>(null) }
    val tag = "SelectedWalletContainer"

    // load manager on appear
    LaunchedEffect(id) {
        // capture the wallet ID we're loading to detect if it changes mid-load
        val requestedId = id

        // clear old state immediately to prevent race conditions
        manager = null
        loadedId = null

        try {
            android.util.Log.d(tag, "getting wallet $requestedId")
            val wm = app.getWalletManager(requestedId)

            // only set manager if we're still loading the same wallet (not stale)
            if (isActive && requestedId == id) {
                manager = wm
                loadedId = requestedId

                // small delay then update balance
                delay(BALANCE_UPDATE_DELAY_MS)
                wm.updateWalletBalance()
            } else {
                android.util.Log.d(tag, "discarding stale wallet load for $requestedId, now loading $id")
            }
        } catch (e: Exception) {
            android.util.Log.e(tag, "something went very wrong", e)

            // try to select another wallet or go to add wallet
            try {
                val wallets = Database().wallets().all()
                val otherWallet = wallets.firstOrNull { it.id != id }

                if (otherWallet != null) {
                    app.rust.selectWallet(otherWallet.id)
                } else {
                    app.loadAndReset(RouteFactory().newWalletSelect())
                }
            } catch (ex: Exception) {
                app.loadAndReset(RouteFactory().newWalletSelect())
            }
        }
    }

    // start wallet scan after loading
    LaunchedEffect(manager) {
        manager?.let { wm ->
            try {
                // small delay and then start scanning wallet
                delay(WALLET_SCAN_DELAY_MS)
                wm.rust.getTransactions()
                wm.updateWalletBalance()
                wm.rust.startWalletScan()
            } catch (e: Exception) {
                android.util.Log.e(tag, "wallet scan failed: ${e.message}", e)
            }
        }
    }

    // cleanup on disappear
    DisposableEffect(Unit) {
        onDispose {
            manager?.dispatch(WalletManagerAction.SelectedWalletDisappeared)
        }
    }

    // update app wallet manager when loaded
    LaunchedEffect(manager?.loadState) {
        val loadState = manager?.loadState
        if (loadState is WalletLoadState.LOADED) {
            manager?.let { app.setWalletManager(it) }
        }
    }

    // state for more options sheet
    var showMoreOptions by remember { mutableStateOf(false) }
    var exportType by remember { mutableStateOf<ExportType?>(null) }
    var isExporting by remember { mutableStateOf(false) }
    var isImporting by remember { mutableStateOf(false) }

    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    val snackbarHostState = remember { SnackbarHostState() }

    // cleanup on dispose - clear alert state if export is in progress
    DisposableEffect(Unit) {
        onDispose {
            if (isExporting && app.alertState != null) {
                app.alertState = null
            }
        }
    }

    // file import launcher (for labels) - accepts plain text and JSON files
    val importLabelLauncher =
        rememberLauncherForActivityResult(ActivityResultContracts.OpenDocument()) { uri ->
            uri?.let {
                scope.launch {
                    isImporting = true
                    try {
                        val fileContents =
                            withContext(Dispatchers.IO) {
                                context.contentResolver.openInputStream(uri)?.use { input ->
                                    input.bufferedReader().use { it.readText() }
                                }
                            } ?: throw Exception("Unable to read file")

                        // validate import was successful before showing success message
                        val labelManager =
                            manager?.rust?.labelManager()
                                ?: throw Exception("Label manager not available")

                        labelManager.import(fileContents.trim())

                        // refresh transactions with updated labels
                        try {
                            manager?.rust?.getTransactions()
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
                        isImporting = false
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
                    isExporting = true
                    val currentExportType = exportType
                    var showedLoadingAlert = false

                    try {
                        // show loading alert for transaction exports after a delay
                        val alertJob =
                            scope.launch {
                                delay(EXPORT_LOADING_ALERT_DELAY_MS)
                                if (isExporting && currentExportType is ExportType.Transactions) {
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
                                        manager?.rust?.labelManager()?.export()
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
                        isExporting = false
                        exportType = null
                    }
                }
            } ?: run {
                // reset state if user cancelled the document picker
                exportType = null
            }
        }

    // render
    when (val wm = manager) {
        null -> FullPageLoadingView(modifier = modifier)
        else -> {
            WalletTransactionsScreen(
                onBack = { app.popRoute() },
                onSend = {
                    app.pushRoute(Route.Send(SendRoute.SetAmount(id, null, null)))
                },
                onReceive = {
                    // TODO: implement receive address screen/sheet
                },
                onQrCode = {
                    // TODO: implement QR code scanner
                },
                onMore = {
                    showMoreOptions = true
                },
                // TODO: get from theme
                isDarkList = false,
                manager = wm,
                snackbarHostState = snackbarHostState,
            )

            // show more options bottom sheet
            if (showMoreOptions) {
                WalletMoreOptionsSheet(
                    app = app,
                    manager = wm,
                    onDismiss = { showMoreOptions = false },
                    onImportLabels = {
                        showMoreOptions = false
                        importLabelLauncher.launch(arrayOf("text/plain", "application/json", "application/x-jsonlines"))
                    },
                    onExportLabels = {
                        showMoreOptions = false
                        exportType = ExportType.Labels
                        val metadata = wm.walletMetadata
                        val fileName = wm.rust.labelManager().exportDefaultFileName(metadata?.name ?: "wallet")
                        exportFileLauncher.launch(fileName)
                    },
                    onExportTransactions = {
                        showMoreOptions = false
                        exportType = ExportType.Transactions
                        val metadata = wm.walletMetadata
                        val fileName = "${metadata?.name?.lowercase() ?: "wallet"}_transactions.csv"
                        exportFileLauncher.launch(fileName)
                    },
                )
            }
        }
    }
}
