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
    var isExporting by remember { mutableStateOf(false) }

    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    val snackbarHostState = remember { SnackbarHostState() }

    // file import launcher (for labels)
    val importLabelLauncher =
        rememberLauncherForActivityResult(ActivityResultContracts.GetContent()) { uri ->
            uri?.let {
                scope.launch {
                    try {
                        val fileContents =
                            withContext(Dispatchers.IO) {
                                context.contentResolver.openInputStream(uri)?.use { input ->
                                    input.bufferedReader().use { it.readText() }
                                }
                            } ?: throw Exception("Unable to read file")

                        manager?.rust?.labelManager()?.import(fileContents.trim())
                        manager?.rust?.getTransactions()

                        snackbarHostState.showSnackbar("Labels imported successfully")
                    } catch (e: Exception) {
                        android.util.Log.e(tag, "error importing labels", e)
                        snackbarHostState.showSnackbar("Error importing labels: ${e.message}")
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
                    try {
                        val content =
                            when {
                                isExporting -> {
                                    withContext(Dispatchers.IO) {
                                        manager?.rust?.createTransactionsWithFiatExport()
                                    }
                                }
                                else -> {
                                    withContext(Dispatchers.IO) {
                                        manager?.rust?.labelManager()?.export()
                                    }
                                }
                            }

                        content?.let { data ->
                            withContext(Dispatchers.IO) {
                                context.contentResolver.openOutputStream(uri)?.use { output ->
                                    output.bufferedWriter().use { it.write(data) }
                                }
                            }

                            val message =
                                if (isExporting) "Transactions exported successfully" else "Labels exported successfully"
                            snackbarHostState.showSnackbar(message)
                        } ?: run {
                            val errorType = if (isExporting) "transactions" else "labels"
                            snackbarHostState.showSnackbar("Error: Unable to generate $errorType export data")
                        }

                        isExporting = false
                    } catch (e: Exception) {
                        android.util.Log.e(tag, "error exporting file", e)
                        snackbarHostState.showSnackbar("Error exporting: ${e.message}")
                        isExporting = false
                    }
                }
            } ?: run {
                // reset flag if user cancelled the document picker
                isExporting = false
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

            // more options bottom sheet
            if (showMoreOptions) {
                WalletMoreOptionsSheet(
                    app = app,
                    manager = wm,
                    onDismiss = { showMoreOptions = false },
                    onImportLabels = {
                        showMoreOptions = false
                        importLabelLauncher.launch("*/*")
                    },
                    onExportLabels = {
                        showMoreOptions = false
                        val metadata = wm.walletMetadata
                        val fileName = wm.rust.labelManager().exportDefaultFileName(metadata?.name ?: "wallet")
                        exportFileLauncher.launch(fileName)
                    },
                    onExportTransactions = {
                        showMoreOptions = false
                        isExporting = true
                        val metadata = wm.walletMetadata
                        val fileName = "${metadata?.name?.lowercase() ?: "wallet"}_transactions.csv"
                        exportFileLauncher.launch(fileName)
                    },
                )
            }
        }
    }
}
