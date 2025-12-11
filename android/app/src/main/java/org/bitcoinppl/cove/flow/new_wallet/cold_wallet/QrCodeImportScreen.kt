package org.bitcoinppl.cove.flow.new_wallet.cold_wallet

import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
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
fun QrCodeImportScreen(app: AppManager, modifier: Modifier = Modifier) {
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
                            style = MaterialTheme.typography.titleLarge,
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
                                Log.d("QrCodeImportScreen", "Imported Wallet: $id")

                                app.rust.selectWallet(id = id)
                                app.alertState =
                                    TaggedItem(
                                        AppAlertState.General(
                                            title = "Success",
                                            message = "Imported Wallet Successfully",
                                        ),
                                    )
                            } catch (e: WalletException.WalletAlreadyExists) {
                                try {
                                    app.rust.selectWallet(id = e.v1)
                                    app.alertState =
                                        TaggedItem(
                                            AppAlertState.General(
                                                title = "Success",
                                                message = "Wallet already exists: ${e.v1}",
                                            ),
                                        )
                                } catch (selectError: Exception) {
                                    Log.w(
                                        "QrCodeImportScreen",
                                        "Unable to select existing wallet",
                                        selectError,
                                    )
                                    app.alertState =
                                        TaggedItem(
                                            AppAlertState.ErrorImportingHardwareWallet(
                                                message = selectError.message ?: "Unable to select wallet",
                                            ),
                                        )
                                }
                            } catch (e: Exception) {
                                Log.w("QrCodeImportScreen", "Error importing hardware wallet: $e")
                                app.alertState =
                                    TaggedItem(
                                        AppAlertState.ErrorImportingHardwareWallet(
                                            message = e.message ?: "Unknown error",
                                        ),
                                    )
                            }
                        }
                        else -> {
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
        HelpSheet(onDismiss = { showHelp = false })
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun HelpSheet(onDismiss: () -> Unit) {
    ModalBottomSheet(onDismissRequest = onDismiss) {
        Column(
            modifier =
                Modifier
                    .fillMaxWidth()
                    .padding(24.dp)
                    .verticalScroll(rememberScrollState()),
            verticalArrangement = Arrangement.spacedBy(24.dp),
        ) {
            Text(
                text = "How do I get my wallet export QR code?",
                style = MaterialTheme.typography.titleLarge,
                fontWeight = FontWeight.Bold,
            )

            Column(verticalArrangement = Arrangement.spacedBy(32.dp)) {
                // ColdCard Q1
                Column(verticalArrangement = Arrangement.spacedBy(12.dp)) {
                    Text(
                        text = "ColdCard Q1",
                        style = MaterialTheme.typography.titleMedium,
                        fontWeight = FontWeight.Bold,
                    )
                    Text("1. Go to 'Advanced / Tools'")
                    Text("2. Export Wallet > Generic JSON")
                    Text("3. Press the 'Enter' button, then the 'QR' button")
                    Text("4. Scan the Generated QR code")
                }

                HorizontalDivider()

                // ColdCard MK3/MK4
                Column(verticalArrangement = Arrangement.spacedBy(12.dp)) {
                    Text(
                        text = "ColdCard MK3/MK4",
                        style = MaterialTheme.typography.titleMedium,
                        fontWeight = FontWeight.Bold,
                    )
                    Text("1. Go to 'Advanced / Tools'")
                    Text("2. Export Wallet > Descriptor")
                    Text("3. Press the Enter (✓) and select your wallet type")
                    Text("4. Scan the Generated QR code")
                }

                HorizontalDivider()

                // Sparrow Desktop
                Column(verticalArrangement = Arrangement.spacedBy(12.dp)) {
                    Text(
                        text = "Sparrow Desktop",
                        style = MaterialTheme.typography.titleMedium,
                        fontWeight = FontWeight.Bold,
                    )
                    Text("1. Click on Settings, in the left side bar")
                    Text("2. Click on 'Export...' button at the bottom")
                    Text("3. Under 'Output Descriptor' click the 'Show...' button")
                    Text("4. Make sure 'Show BBQr' is selected")
                    Text("5. Scan the generated QR code")
                }

                HorizontalDivider()

                // Other Hardware Wallets
                Column(verticalArrangement = Arrangement.spacedBy(12.dp)) {
                    Text(
                        text = "Other Hardware Wallets",
                        style = MaterialTheme.typography.titleMedium,
                        fontWeight = FontWeight.Bold,
                    )
                    Text("1. In your hardware wallet, go to settings")
                    Text("2. Look for 'Export'")
                    Text("3. Select 'Generic JSON', 'Sparrow', 'Electrum', and many other formats should also work")
                    Text("4. Generate QR code")
                    Text("5. Scan the Generated QR code")
                }
            }

            Spacer(modifier = Modifier.height(24.dp))
        }
    }
}
