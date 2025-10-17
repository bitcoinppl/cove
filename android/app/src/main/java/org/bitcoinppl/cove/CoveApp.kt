package org.bitcoinppl.cove

import androidx.activity.compose.BackHandler
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Button
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import org.bitcoinppl.cove.sheets.QrScannerSheet
import org.bitcoinppl.cove.ui.theme.CoveTheme
import uniffi.cove_core_ffi.MultiFormat

/**
 * root Compose application
 * manages app initialization, auth, terms acceptance, and navigation
 * ported from iOS CoveApp.swift
 */
@Composable
fun CoveApp() {
    val app = remember { AppManager.getInstance() }
    val auth = remember { AuthManager.getInstance() }

    // initialize app on start
    LaunchedEffect(Unit) {
        app.rust.initOnStart()
    }

    CoveTheme {
        Surface(
            modifier = Modifier.fillMaxSize(),
            color = MaterialTheme.colorScheme.background
        ) {
            when {
                // show lock screen if auth is enabled and locked
                auth.isAuthEnabled && auth.lockState == LockState.LOCKED -> {
                    LockScreen()
                }
                // show terms screen if not accepted
                !app.isTermsAccepted -> {
                    TermsScreen(onAccept = { app.agreeToTerms() })
                }
                // show loading if needed
                app.isLoading -> {
                    LoadingScreen()
                }
                // show main app
                else -> {
                    MainAppContent(app = app)
                }
            }

            // global alerts
            app.alertState?.let { taggedAlert ->
                AlertDialog(
                    onDismissRequest = { app.alertState = null },
                    title = { Text(taggedAlert.item.title()) },
                    text = { Text(getAlertMessage(taggedAlert.item)) },
                    confirmButton = {
                        TextButton(onClick = { app.alertState = null }) {
                            Text("OK")
                        }
                    }
                )
            }

            // global sheets
            app.sheetState?.let { taggedSheet ->
                when (taggedSheet.item) {
                    is AppSheetState.Qr -> {
                        QrScannerSheet(
                            app = app,
                            onScanned = { scannedData ->
                                app.sheetState = null
                                handleMultiFormat(app, scannedData)
                            },
                            onDismiss = {
                                app.sheetState = null
                            }
                        )
                    }
                    is AppSheetState.TapSigner -> {
                        // TODO: implement TapSigner sheet in Phase 6
                        app.sheetState = null
                    }
                }
            }
        }
    }
}

@Composable
private fun MainAppContent(app: AppManager) {
    // hardware back button handling - route through Rust
    BackHandler(enabled = app.router.routes.isNotEmpty()) {
        app.popRoute()
    }

    // use routeId as key to force recomposition when route resets
    // this ensures view lifecycle is properly reset when default route changes
    Box(modifier = Modifier.fillMaxSize(), key = app.routeId) {
        RouteView(app = app, route = app.currentRoute)
    }
}

@Composable
private fun LockScreen() {
    Box(modifier = Modifier.fillMaxSize()) {
        // TODO: implement proper lock screen with PIN entry
        Text("Lock Screen - TODO")
    }
}

@Composable
private fun TermsScreen(onAccept: () -> Unit) {
    Box(modifier = Modifier.fillMaxSize()) {
        // TODO: implement proper terms screen
        Button(onClick = onAccept) {
            Text("Accept Terms (TODO: Show actual terms)")
        }
    }
}

@Composable
private fun LoadingScreen() {
    Box(
        modifier = Modifier.fillMaxSize(),
        contentAlignment = Alignment.Center
    ) {
        CircularProgressIndicator()
    }
}

/**
 * get alert message text based on alert state
 */
private fun getAlertMessage(alert: AppAlertState): String = when (alert) {
    is AppAlertState.ImportedSuccessfully -> "Wallet imported successfully"
    is AppAlertState.ImportedLabelsSuccessfully -> "Labels imported successfully"
    is AppAlertState.DuplicateWallet -> "This wallet has already been imported"
    is AppAlertState.InvalidWordGroup -> "The recovery words entered are not valid"
    is AppAlertState.ErrorImportingHotWallet -> alert.error
    is AppAlertState.AddressWrongNetwork -> "Address is for ${alert.network} but current network is ${alert.currentNetwork}"
    is AppAlertState.FoundAddress -> "Found address: ${alert.address}"
    is AppAlertState.UnableToSelectWallet -> "Unable to select wallet"
    is AppAlertState.ErrorImportingHardwareWallet -> alert.error
    is AppAlertState.InvalidFileFormat -> "Invalid file format: ${alert.format}"
    is AppAlertState.NoWalletSelected -> "No wallet selected for address: ${alert.address}"
    is AppAlertState.InvalidFormat -> "Invalid format: ${alert.format}"
    is AppAlertState.NoUnsignedTransactionFound -> "No unsigned transaction found"
    is AppAlertState.UnableToGetAddress -> alert.error
    is AppAlertState.NoCameraPermission -> "Camera permission is required to scan QR codes"
    is AppAlertState.FailedToScanQr -> alert.error
    is AppAlertState.CantSendOnWatchOnlyWallet -> "Cannot send from watch-only wallet"
    is AppAlertState.TapSignerSetupFailed -> alert.error
    is AppAlertState.TapSignerDeriveFailed -> alert.error
    is AppAlertState.TapSignerInvalidAuth -> "Invalid PIN entered"
    is AppAlertState.TapSignerNoBackup -> "No backup found for this TAPSIGNER"
    is AppAlertState.General -> alert.message
    is AppAlertState.UninitializedTapSigner -> "Would you like to setup this TAPSIGNER?"
    is AppAlertState.TapSignerWalletFound -> "Wallet found on TAPSIGNER"
    is AppAlertState.InitializedTapSigner -> "Would you like to import this TAPSIGNER?"
}

/**
 * handle scanned QR code multiformat data
 * ported from iOS CoveApp.swift handleMultiFormat
 */
private fun handleMultiFormat(app: AppManager, scannedData: uniffi.cove_core_ffi.StringOrData) {
    try {
        val multiFormat = scannedData.toMultiFormat()

        when (multiFormat) {
            is MultiFormat.Mnemonic -> {
                // TODO: implement hot wallet import
                android.util.Log.d("CoveApp", "Scanned mnemonic, import not yet implemented")
                app.alertState = TaggedItem(
                    AppAlertState.General(
                        "Wallet Import",
                        "Hot wallet import from QR not yet implemented"
                    )
                )
            }
            is MultiFormat.HardwareExport -> {
                // TODO: implement hardware wallet import
                android.util.Log.d("CoveApp", "Scanned hardware export, import not yet implemented")
                app.alertState = TaggedItem(
                    AppAlertState.General(
                        "Hardware Wallet",
                        "Hardware wallet import from QR not yet implemented"
                    )
                )
            }
            is MultiFormat.Address -> {
                // handle address - show alert with send option if wallet selected
                val addressWithNetwork = multiFormat.address
                val address = addressWithNetwork.address()
                val amount = addressWithNetwork.amount()

                app.alertState = TaggedItem(
                    AppAlertState.FoundAddress(address, amount)
                )
            }
            is MultiFormat.Transaction -> {
                // TODO: handle transaction
                android.util.Log.d("CoveApp", "Scanned transaction, handling not yet implemented")
                app.alertState = TaggedItem(
                    AppAlertState.General(
                        "Transaction",
                        "Transaction handling from QR not yet implemented"
                    )
                )
            }
            is MultiFormat.TapSignerUnused -> {
                app.alertState = TaggedItem(
                    AppAlertState.UninitializedTapSigner(multiFormat.tapSigner)
                )
            }
            is MultiFormat.TapSignerReady -> {
                // TODO: check if wallet exists for this TapSigner
                app.alertState = TaggedItem(
                    AppAlertState.InitializedTapSigner(multiFormat.tapSigner)
                )
            }
            is MultiFormat.Bip329Labels -> {
                // TODO: implement label import
                android.util.Log.d("CoveApp", "Scanned BIP329 labels, import not yet implemented")
                app.alertState = TaggedItem(
                    AppAlertState.General(
                        "Labels",
                        "Label import from QR not yet implemented"
                    )
                )
            }
        }
    } catch (e: Exception) {
        android.util.Log.e("CoveApp", "Error handling scanned QR: ${e.message}")
        app.alertState = TaggedItem(
            AppAlertState.General(
                "QR Scan Error",
                e.message ?: "Unknown error processing QR code"
            )
        )
    }
}
