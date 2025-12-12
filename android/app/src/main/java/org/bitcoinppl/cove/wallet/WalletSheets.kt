package org.bitcoinppl.cove.wallet

import androidx.compose.material3.SnackbarHostState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.rememberCoroutineScope
import kotlinx.coroutines.launch
import org.bitcoinppl.cove.AppManager
import org.bitcoinppl.cove.WalletManager
import org.bitcoinppl.cove.nfc.NfcLabelImportSheet
import org.bitcoinppl.cove.wallet_transactions.ReceiveAddressSheet
import org.bitcoinppl.cove.wallet_transactions.WalletMoreOptionsSheet

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
    if (showNfcScanner) {
        val labelManager =
            try {
                manager.rust.labelManager()
            } catch (e: Exception) {
                android.util.Log.e(tag, "Failed to get label manager", e)
                onDismissNfcScanner()
                scope.launch {
                    snackbarHostState.showSnackbar("Unable to access label manager")
                }
                null
            }

        labelManager?.let {
            NfcLabelImportSheet(
                labelManager = it,
                onDismiss = onDismissNfcScanner,
                onSuccess = {
                    onDismissNfcScanner()
                    scope.launch {
                        // refresh transactions with updated labels
                        try {
                            manager.rust.getTransactions()
                            snackbarHostState.showSnackbar("Labels imported successfully")
                        } catch (e: Exception) {
                            android.util.Log.e(tag, "Failed to refresh transactions after NFC label import", e)
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
