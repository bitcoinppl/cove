package org.bitcoinppl.cove.flows.NewWalletFlow.cold_wallet

import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import org.bitcoinppl.cove.AppManager
import org.bitcoinppl.cove.Log
import org.bitcoinppl.cove.QrCodeScanView
import org.bitcoinppl.cove.R
import org.bitcoinppl.cove.TaggedItem
import org.bitcoinppl.cove_core.AppAlertState
import org.bitcoinppl.cove_core.MultiFormat
import org.bitcoinppl.cove_core.Wallet
import org.bitcoinppl.cove_core.WalletException

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun QrCodeImportScreen(app: AppManager, modifier: Modifier = Modifier) {
    var showHelp by remember { mutableStateOf(false) }
    val context = LocalContext.current

    Scaffold(
        topBar = {
            TopAppBar(
                title = { },
                navigationIcon = {
                    IconButton(onClick = { app.popRoute() }) {
                        Icon(
                            Icons.AutoMirrored.Filled.ArrowBack,
                            contentDescription = stringResource(R.string.new_wallet_flow_back),
                            tint = Color.White,
                        )
                    }
                },
                actions = {
                    TextButton(onClick = { showHelp = true }) {
                        Text(
                            text = stringResource(R.string.new_wallet_flow_help_button),
                            color = Color.White,
                            style = MaterialTheme.typography.bodyLarge,
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

                                app.selectWalletOrThrow(id)
                                app.popRoute()
                                app.alertState = TaggedItem(AppAlertState.ImportedSuccessfully)
                            } catch (e: WalletException.WalletAlreadyExists) {
                                app.popRoute()
                                app.alertState = TaggedItem(AppAlertState.DuplicateWallet(e.v1))
                            } catch (e: Exception) {
                                Log.w("QrCodeImportScreen", "Error importing hardware wallet: $e")
                                app.popRoute()
                                app.alertState =
                                    TaggedItem(
                                        AppAlertState.ErrorImportingHardwareWallet(
                                            message = context.getString(R.string.app_alert_error_importing_hardware_message),
                                        ),
                                    )
                            }
                        }
                        else -> {
                            app.popRoute()
                            app.alertState =
                                TaggedItem(
                                    AppAlertState.General(
                                        title = context.getString(R.string.new_wallet_flow_invalid_qr_code),
                                        message = context.getString(R.string.new_wallet_flow_invalid_hardware_qr),
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
                text = stringResource(R.string.new_wallet_flow_wallet_export_qr_help_title),
                style = MaterialTheme.typography.headlineMedium,
                fontWeight = FontWeight.Bold,
            )

            Column(verticalArrangement = Arrangement.spacedBy(32.dp)) {
                // ColdCard Q1
                Column(verticalArrangement = Arrangement.spacedBy(12.dp)) {
                    Text(
                        text = stringResource(R.string.new_wallet_flow_coldcard_q1),
                        style = MaterialTheme.typography.titleLarge,
                        fontWeight = FontWeight.Bold,
                    )
                    Text(stringResource(R.string.new_wallet_flow_qr_help_advanced_tools))
                    Text(stringResource(R.string.new_wallet_flow_qr_help_export_generic_json))
                    Text(stringResource(R.string.new_wallet_flow_qr_help_press_enter_qr))
                    Text(stringResource(R.string.new_wallet_flow_qr_help_scan_generated_qr))
                }

                HorizontalDivider()

                // ColdCard MK3/MK4
                Column(verticalArrangement = Arrangement.spacedBy(12.dp)) {
                    Text(
                        text = stringResource(R.string.new_wallet_flow_coldcard_mk3_mk4),
                        style = MaterialTheme.typography.titleLarge,
                        fontWeight = FontWeight.Bold,
                    )
                    Text(stringResource(R.string.new_wallet_flow_qr_help_advanced_tools))
                    Text(stringResource(R.string.new_wallet_flow_qr_help_export_descriptor))
                    Text(stringResource(R.string.new_wallet_flow_qr_help_press_enter_select_wallet_type))
                    Text(stringResource(R.string.new_wallet_flow_qr_help_scan_generated_qr))
                }

                HorizontalDivider()

                // Sparrow Desktop
                Column(verticalArrangement = Arrangement.spacedBy(12.dp)) {
                    Text(
                        text = stringResource(R.string.new_wallet_flow_sparrow_desktop),
                        style = MaterialTheme.typography.titleLarge,
                        fontWeight = FontWeight.Bold,
                    )
                    Text(stringResource(R.string.new_wallet_flow_qr_help_sparrow_settings))
                    Text(stringResource(R.string.new_wallet_flow_qr_help_sparrow_export))
                    Text(stringResource(R.string.new_wallet_flow_qr_help_sparrow_show_descriptor))
                    Text(stringResource(R.string.new_wallet_flow_qr_help_sparrow_show_bbqr))
                    Text(stringResource(R.string.new_wallet_flow_qr_help_scan_generated_qr_lower))
                }

                HorizontalDivider()

                // Other Hardware Wallets
                Column(verticalArrangement = Arrangement.spacedBy(12.dp)) {
                    Text(
                        text = stringResource(R.string.new_wallet_flow_other_hardware_wallets),
                        style = MaterialTheme.typography.titleLarge,
                        fontWeight = FontWeight.Bold,
                    )
                    Text(stringResource(R.string.new_wallet_flow_qr_help_hardware_settings))
                    Text(stringResource(R.string.new_wallet_flow_qr_help_look_for_export))
                    Text(stringResource(R.string.new_wallet_flow_qr_help_select_export_format))
                    Text(stringResource(R.string.new_wallet_flow_qr_help_generate_qr))
                    Text(stringResource(R.string.new_wallet_flow_qr_help_scan_generated_qr_step_5))
                }
            }

            Spacer(modifier = Modifier.height(24.dp))
        }
    }
}
