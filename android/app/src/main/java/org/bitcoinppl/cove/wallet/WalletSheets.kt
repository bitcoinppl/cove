package org.bitcoinppl.cove.wallet

import androidx.compose.material3.SnackbarHostState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import kotlinx.coroutines.launch
import org.bitcoinppl.cove.AppManager
import org.bitcoinppl.cove.WalletManager
import org.bitcoinppl.cove.nfc.NfcLabelImportSheet
import org.bitcoinppl.cove.wallet_transactions.ReceiveAddressSheet
import org.bitcoinppl.cove.wallet_transactions.WalletMoreOptionsSheet
import org.bitcoinppl.cove_core.LabelManager

@Composable
internal fun WalletSheetsHost(
    app: AppManager,
    manager: WalletManager,
    snackbarHostState: SnackbarHostState,
    showMoreOptions: Boolean,
    showReceiveSheet: Boolean,
    showNfcScanner: Boolean,
    exportLaunchers: WalletExportLaunchers,
    onDismissMoreOptions: () -> Unit,
    onDismissReceiveSheet: () -> Unit,
    onDismissNfcScanner: () -> Unit,
    onShowNfcScanner: () -> Unit,
    tag: String = "WalletSheets",
) {
    val scope = rememberCoroutineScope()

    // show more options bottom sheet
    if (showMoreOptions) {
        WalletMoreOptionsSheet(
            app = app,
            manager = manager,
            onDismiss = onDismissMoreOptions,
            onScanNfc = {
                onDismissMoreOptions()
                onShowNfcScanner()
            },
            onImportLabels = {
                onDismissMoreOptions()
                exportLaunchers.importLabels()
            },
            onExportLabels = {
                onDismissMoreOptions()
                val metadata = manager.walletMetadata
                val fileName =
                    manager.rust.labelManager().use { lm ->
                        lm.exportDefaultFileName(metadata?.name ?: "wallet")
                    }
                exportLaunchers.exportLabels(fileName)
            },
            onExportTransactions = {
                onDismissMoreOptions()
                val metadata = manager.walletMetadata
                val fileName = "${metadata?.name?.lowercase() ?: "wallet"}_transactions.csv"
                exportLaunchers.exportTransactions(fileName)
            },
        )
    }

    // show NFC label import sheet
    var nfcLabelManager by remember { mutableStateOf<LabelManager?>(null) }

    LaunchedEffect(showNfcScanner) {
        if (showNfcScanner) {
            try {
                nfcLabelManager = manager.rust.labelManager()
            } catch (e: Exception) {
                android.util.Log.e(tag, "Failed to get label manager")
                nfcLabelManager = null
                onDismissNfcScanner()
                snackbarHostState.showSnackbar("Unable to access label manager")
            }
        } else {
            nfcLabelManager?.close()
            nfcLabelManager = null
        }
    }

    DisposableEffect(Unit) {
        onDispose {
            nfcLabelManager?.close()
        }
    }

    if (showNfcScanner) {
        nfcLabelManager?.let { labelManager ->
            NfcLabelImportSheet(
                labelManager = labelManager,
                onDismiss = onDismissNfcScanner,
                onSuccess = {
                    onDismissNfcScanner()
                    scope.launch {
                        // refresh transactions with updated labels
                        try {
                            manager.rust.getTransactions()
                            snackbarHostState.showSnackbar("Labels imported successfully")
                        } catch (e: Exception) {
                            android.util.Log.e(tag, "Failed to refresh transactions after NFC label import")
                            snackbarHostState.showSnackbar("Labels imported, but failed to refresh transactions")
                        }
                    }
                },
                onError = { errorMsg ->
                    onDismissNfcScanner()
                    scope.launch {
                        snackbarHostState.showSnackbar("Failed to import labels: $errorMsg")
                    }
                },
            )
        }
    }

    // show receive address sheet
    if (showReceiveSheet) {
        ReceiveAddressSheet(
            manager = manager,
            snackbarHostState = snackbarHostState,
            onDismiss = onDismissReceiveSheet,
        )
    }
}
