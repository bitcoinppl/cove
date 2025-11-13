package org.bitcoinppl.cove.flow.new_wallet.cold_wallet

import android.util.Log
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
import org.bitcoinppl.cove.QrCodeScanView
import org.bitcoinppl.cove.TaggedItem
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
                onScanned = { stringOrData ->
                    try {
                        // try to parse as string first for xpub/descriptor
                        val xpub =
                            when (stringOrData) {
                                is org.bitcoinppl.cove_core.StringOrData.String -> stringOrData.v1
                                is org.bitcoinppl.cove_core.StringOrData.Data ->
                                    stringOrData.v1.toString(Charsets.UTF_8)
                            }

                        val wallet = Wallet.newFromXpub(xpub = xpub)
                        val id = wallet.id()
                        Log.d("ColdWalletQrScanScreen", "Imported Wallet: $id")

                        app.rust.selectWallet(id = id)
                        app.popRoute()
                        app.alertState =
                            TaggedItem(
                                AppAlertState.General(
                                    title = "Success",
                                    message = "Imported Wallet Successfully",
                                ),
                            )
                    } catch (e: WalletException.MultiFormat) {
                        app.popRoute()
                        app.alertState =
                            TaggedItem(
                                AppAlertState.ErrorImportingHardwareWallet(
                                    message = e.v1.toString(),
                                ),
                            )
                    } catch (e: WalletException.WalletAlreadyExists) {
                        try {
                            app.rust.selectWallet(id = e.v1)
                            app.popRoute()
                            app.alertState =
                                TaggedItem(
                                    AppAlertState.General(
                                        title = "Success",
                                        message = "Wallet already exists: ${e.v1}",
                                    ),
                                )
                        } catch (selectError: Exception) {
                            app.popRoute()
                            app.alertState =
                                TaggedItem(
                                    AppAlertState.ErrorImportingHardwareWallet(
                                        message = "Unable to select wallet",
                                    ),
                                )
                        }
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
                },
                onDismiss = { app.popRoute() },
                modifier = Modifier.fillMaxSize(),
            )
        }
    }

    if (showHelp) {
        WalletImportHelpSheet(onDismiss = { showHelp = false })
    }
}
