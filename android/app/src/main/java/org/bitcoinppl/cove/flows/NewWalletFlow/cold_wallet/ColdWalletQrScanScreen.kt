package org.bitcoinppl.cove.flows.NewWalletFlow.cold_wallet

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.material3.TopAppBarDefaults
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.sp
import org.bitcoinppl.cove.AppAlertState
import org.bitcoinppl.cove.AppManager
import org.bitcoinppl.cove.Log
import org.bitcoinppl.cove.QrCodeScanView
import org.bitcoinppl.cove.TaggedItem
import org.bitcoinppl.cove_core.MultiFormat
import org.bitcoinppl.cove_core.Wallet
import org.bitcoinppl.cove_core.WalletException

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun ColdWalletQrScanScreen(app: AppManager, modifier: Modifier = Modifier) {
    var showHelp by remember { mutableStateOf(false) }

    Scaffold(
        topBar = {
            TopAppBar(
                title = { },
                navigationIcon = {
                    IconButton(onClick = { app.popRoute() }) {
                        Icon(
                            Icons.AutoMirrored.Filled.ArrowBack,
                            contentDescription = "Back",
                            tint = Color.White,
                        )
                    }
                },
                actions = {
                    TextButton(onClick = { showHelp = true }) {
                        Text(
                            text = "?",
                            color = Color.White,
                            fontSize = 24.sp,
                            fontWeight = FontWeight.Medium,
                        )
                    }
                },
                colors =
                    TopAppBarDefaults.topAppBarColors(
                        containerColor = Color.Transparent,
                    ),
            )
        },
        containerColor = Color.Black,
        modifier = modifier.fillMaxSize(),
    ) { paddingValues ->
        Box(modifier = Modifier.fillMaxSize().padding(paddingValues)) {
            QrCodeScanView(
                showTopBar = false,
                onScanned = { multiFormat ->
                    when (multiFormat) {
                        is MultiFormat.HardwareExport -> {
                            try {
                                val wallet = Wallet.newFromExport(export = multiFormat.v1)
                                val id = wallet.id()
                                wallet.close()
                                Log.d("ColdWalletQrScanScreen", "Imported Wallet: $id")

                                app.rust.selectWallet(id = id)
                                app.popRoute()
                                app.alertState = TaggedItem(AppAlertState.ImportedSuccessfully)
                            } catch (e: WalletException.WalletAlreadyExists) {
                                app.popRoute()
                                app.alertState = TaggedItem(AppAlertState.DuplicateWallet(e.v1))
                            } catch (e: Exception) {
                                Log.w("ColdWalletQrScanScreen", "Error importing hardware wallet: $e")
                                app.popRoute()
                                app.alertState =
                                    TaggedItem(
                                        AppAlertState.ErrorImportingHardwareWallet(
                                            message = e.message ?: "Unknown error",
                                        ),
                                    )
                            }
                        }
                        else -> {
                            app.popRoute()
                            app.alertState =
                                TaggedItem(
                                    AppAlertState.General(
                                        title = "Invalid QR Code",
                                        message = "Please scan a valid hardware wallet export QR code",
                                    ),
                                )
                        }
                    }
                },
                onDismiss = { app.popRoute() },
                app = app,
                modifier = Modifier.fillMaxSize(),
            )
        }
    }

    if (showHelp) {
        WalletImportHelpSheet(onDismiss = { showHelp = false })
    }
}
