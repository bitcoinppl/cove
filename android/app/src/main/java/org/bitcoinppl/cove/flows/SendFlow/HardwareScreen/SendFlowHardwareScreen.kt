@file:Suppress("PackageNaming")

package org.bitcoinppl.cove.flows.SendFlow.HardwareScreen

import android.content.ClipData
import android.content.ClipboardManager
import android.content.Context
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Delete
import androidx.compose.material3.CenterAlignedTopAppBar
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBarDefaults
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.layout.onSizeChanged
import androidx.compose.ui.platform.LocalConfiguration
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import org.bitcoinppl.cove.AppManager
import org.bitcoinppl.cove.Log
import org.bitcoinppl.cove.R
import org.bitcoinppl.cove.WalletManager
import org.bitcoinppl.cove.ui.theme.CoveColor
import org.bitcoinppl.cove.ui.theme.ForceLightStatusBarIcons
import org.bitcoinppl.cove.ui.theme.title3
import org.bitcoinppl.cove.views.AutoSizeText
import org.bitcoinppl.cove_core.AppAction
import org.bitcoinppl.cove_core.types.ConfirmDetails

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun SendFlowHardwareScreen(
    app: AppManager,
    walletManager: WalletManager,
    details: ConfirmDetails,
    modifier: Modifier = Modifier,
) {
    val context = LocalContext.current

    var sheetState by remember { mutableStateOf<HardwareSheetState?>(null) }
    var confirmationState by remember { mutableStateOf<ConfirmationState?>(null) }
    var alertState by remember { mutableStateOf<AlertState?>(null) }
    var alertMessage by remember { mutableStateOf("") }
    var showQrScanner by remember { mutableStateOf(false) }
    var showNfcWriteSheet by remember { mutableStateOf(false) }

    val metadata = walletManager.walletMetadata

    var fiatAmount by remember { mutableStateOf("---") }
    LaunchedEffect(app.prices) {
        app.prices?.let { prices ->
            val amount = details.sendingAmount()
            fiatAmount = walletManager.convertAndDisplayFiat(amount, prices)
        } ?: run {
            app.dispatch(AppAction.UpdateFiatPrices)
        }
    }

    val filePickerLauncher =
        rememberSignedImportFilePicker(
            app = app,
            context = context,
            onError = { message ->
                alertState = AlertState.FileError
                alertMessage = message
            },
        )

    ForceLightStatusBarIcons()

    Scaffold(
        containerColor = CoveColor.midnightBlue,
        topBar = {
            CenterAlignedTopAppBar(
                colors =
                    TopAppBarDefaults.topAppBarColors(
                        containerColor = Color.Transparent,
                        navigationIconContentColor = Color.White,
                        actionIconContentColor = Color.White,
                    ),
                title = { },
                actions = {
                    IconButton(onClick = {
                        try {
                            walletManager.deleteUnsignedTransaction(details.id())
                            app.popRoute()
                        } catch (e: Exception) {
                            Log.e(
                                "HardwareExport",
                                "Unable to delete transaction ${details.id()}: $e",
                            )
                        }
                    }) {
                        Icon(
                            Icons.Default.Delete,
                            contentDescription = "Delete",
                            tint = Color.White,
                        )
                    }
                },
            )
        },
    ) { paddingValues ->
        Box(
            modifier =
                modifier
                    .fillMaxSize()
                    .padding(paddingValues),
        ) {
            Image(
                painter = painterResource(id = R.drawable.image_chain_code_pattern_horizontal),
                contentDescription = null,
                contentScale = ContentScale.Crop,
                modifier =
                    Modifier
                        .fillMaxHeight()
                        .align(Alignment.TopCenter)
                        .offset(y = (-40).dp)
                        .graphicsLayer(alpha = 0.25f),
            )

            Column(modifier = Modifier.fillMaxSize()) {
                val configuration = LocalConfiguration.current
                val screenHeightDp = configuration.screenHeightDp.dp
                val headerHeight = screenHeightDp * 0.145f

                BalanceHeader(
                    walletManager = walletManager,
                    height = headerHeight,
                )

                Column(
                    modifier =
                        Modifier
                            .fillMaxWidth()
                            .weight(1f)
                            .background(MaterialTheme.colorScheme.surface)
                            .padding(horizontal = 16.dp),
                ) {
                    Column(
                        modifier =
                            Modifier
                                .weight(1f)
                                .verticalScroll(rememberScrollState()),
                        verticalArrangement = Arrangement.spacedBy(24.dp),
                    ) {
                        Column(modifier = Modifier.padding(top = 16.dp)) {
                            Text(
                                text = "You're sending",
                                style = MaterialTheme.typography.titleMedium,
                                fontWeight = FontWeight.Bold,
                                color = MaterialTheme.colorScheme.onSurface,
                                modifier = Modifier.padding(top = 6.dp),
                            )

                            Text(
                                text = "The amount they will receive",
                                style = MaterialTheme.typography.bodySmall,
                                color = MaterialTheme.colorScheme.onSurface.copy(alpha = 0.8f),
                                fontWeight = FontWeight.Medium,
                            )
                        }

                        var unitLabelWidth by remember { mutableStateOf(0.dp) }
                        val density = LocalDensity.current

                        Column(
                            modifier =
                                Modifier
                                    .fillMaxWidth()
                                    .padding(top = 8.dp),
                            horizontalAlignment = Alignment.CenterHorizontally,
                        ) {
                            Row(
                                verticalAlignment = Alignment.Bottom,
                                modifier = Modifier.offset(x = unitLabelWidth / 2),
                            ) {
                                AutoSizeText(
                                    text = walletManager.amountFmt(details.sendingAmount()),
                                    maxFontSize = 48.sp,
                                    minimumScaleFactor = 0.5f,
                                    fontWeight = FontWeight.Bold,
                                    color = MaterialTheme.colorScheme.onSurface,
                                )

                                Text(
                                    text = if (metadata?.selectedUnit?.name == "SAT") "sats" else "btc",
                                    color = MaterialTheme.colorScheme.onSurface,
                                    modifier =
                                        Modifier
                                            .padding(start = 8.dp, bottom = 10.dp)
                                            .onSizeChanged { size ->
                                                unitLabelWidth = with(density) { size.width.toDp() }
                                            },
                                )
                            }

                            Text(
                                text = fiatAmount,
                                style = MaterialTheme.typography.title3,
                                color = MaterialTheme.colorScheme.onSurface.copy(alpha = 0.6f),
                            )
                        }

                        AccountSection(metadata)

                        HorizontalDivider()

                        AddressSection(
                            address = details.sendingTo().spacedOut(),
                            onCopy = {
                                val clipboard =
                                    context.getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager
                                val clip = ClipData.newPlainText("address", details.sendingTo().unformatted())
                                clipboard.setPrimaryClip(clip)
                            },
                            onClick = { sheetState = HardwareSheetState.AdvancedDetails },
                        )

                        HorizontalDivider()

                        HardwareSigningSection(
                            app = app,
                            metadata = metadata,
                            details = details,
                            onExport = { confirmationState = ConfirmationState.ExportTxn },
                            onImport = { confirmationState = ConfirmationState.ImportSignature },
                        )

                        Spacer(modifier = Modifier.height(16.dp))
                    }

                    TextButton(
                        onClick = { sheetState = HardwareSheetState.Details },
                        modifier =
                            Modifier
                                .align(Alignment.CenterHorizontally)
                                .padding(vertical = 16.dp),
                    ) {
                        Text(
                            text = "More details",
                            style = MaterialTheme.typography.bodySmall,
                            color = MaterialTheme.colorScheme.onSurface.copy(alpha = 0.6f),
                            fontWeight = FontWeight.Medium,
                        )
                    }
                }
            }
        }
    }

    SendFlowHardwareBottomSheets(
        app = app,
        walletManager = walletManager,
        details = details,
        sheetState = sheetState,
        onSheetStateChange = { sheetState = it },
    )

    HardwareConfirmationDialogs(
        app = app,
        context = context,
        details = details,
        confirmationState = confirmationState,
        onConfirmationStateChange = { confirmationState = it },
        onShowExportQr = { sheetState = HardwareSheetState.ExportQr },
        onShowQrScanner = { showQrScanner = true },
        onShowNfcWriteSheet = { showNfcWriteSheet = true },
        onLaunchFileImport = { filePickerLauncher.launch("*/*") },
        onAlert = { state, message ->
            alertState = state
            alertMessage = message
        },
    )

    HardwareErrorAlert(
        alertState = alertState,
        alertMessage = alertMessage,
        onDismiss = { alertState = null },
    )

    HardwareQrScanner(
        app = app,
        showQrScanner = showQrScanner,
        onDismiss = { showQrScanner = false },
    )

    HardwareNfcWriteSheet(
        details = details,
        showNfcWriteSheet = showNfcWriteSheet,
        onDismiss = { showNfcWriteSheet = false },
    )
}
