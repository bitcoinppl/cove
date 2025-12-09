package org.bitcoinppl.cove.send

import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.filled.ArrowDropDown
import androidx.compose.material.icons.filled.CurrencyBitcoin
import androidx.compose.material.icons.filled.QrCode2
import androidx.compose.material.icons.filled.Visibility
import androidx.compose.material.icons.filled.VisibilityOff
import androidx.compose.material3.*
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import org.bitcoinppl.cove.AppAlertState
import org.bitcoinppl.cove.AppManager
import org.bitcoinppl.cove.QrCodeScanView
import org.bitcoinppl.cove.R
import org.bitcoinppl.cove.TaggedItem
import org.bitcoinppl.cove.sheets.FeeRateSelectorSheet
import org.bitcoinppl.cove.ui.theme.CoveColor
import org.bitcoinppl.cove.views.AutoSizeText
import org.bitcoinppl.cove.views.BalanceAutoSizeText
import org.bitcoinppl.cove.views.ImageButton
import org.bitcoinppl.cove_core.MultiFormat
import org.bitcoinppl.cove_core.SendFlowManagerAction

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun CoinControlSetAmountScreen(
    onBack: () -> Unit,
    onNext: () -> Unit,
    onScanQr: () -> Unit,
    onChangeSpeed: () -> Unit,
    onToggleBalanceVisibility: () -> Unit = {},
    onAmountTap: () -> Unit,
    onUtxoDetailsClick: () -> Unit,
    isBalanceHidden: Boolean = false,
    balanceAmount: String,
    balanceDenomination: String,
    sendingAmount: String,
    sendingDenomination: String,
    dollarEquivalentText: String,
    initialAddress: String,
    accountShort: String,
    feeEta: String,
    feeAmount: String,
    totalSpendingCrypto: String,
    totalSpendingFiat: String,
    utxoCount: Int,
    utxos: List<org.bitcoinppl.cove_core.types.Utxo>,
    app: AppManager,
    sendFlowManager: org.bitcoinppl.cove.SendFlowManager,
    walletManager: org.bitcoinppl.cove.WalletManager,
    presenter: org.bitcoinppl.cove.SendFlowPresenter,
    onAddressChanged: (String) -> Unit,
    snackbarHostState: SnackbarHostState = remember { SnackbarHostState() },
) {
    Scaffold(
        containerColor = CoveColor.midnightBlue,
        topBar = {
            CenterAlignedTopAppBar(
                colors =
                    TopAppBarDefaults.centerAlignedTopAppBarColors(
                        containerColor = Color.Transparent,
                        titleContentColor = Color.White,
                        navigationIconContentColor = Color.White,
                    ),
                title = { Text("") },
                navigationIcon = {
                    IconButton(onClick = onBack) {
                        Icon(Icons.AutoMirrored.Filled.ArrowBack, contentDescription = null)
                    }
                },
            )
        },
        snackbarHost = { SnackbarHost(snackbarHostState) },
    ) { padding ->
        Box(
            modifier =
                Modifier
                    .fillMaxSize()
                    .padding(padding),
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
                BalanceWidget(
                    amount = balanceAmount,
                    denomination = balanceDenomination,
                    isHidden = isBalanceHidden,
                    onToggleVisibility = onToggleBalanceVisibility,
                )
                Column(
                    modifier =
                        Modifier
                            .fillMaxWidth()
                            .weight(1f)
                            .clip(RoundedCornerShape(topStart = 0.dp, topEnd = 0.dp))
                            .background(MaterialTheme.colorScheme.surface)
                            .verticalScroll(rememberScrollState())
                            .padding(horizontal = 16.dp),
                ) {
                    CoinControlAmountWidget(
                        amount = sendingAmount,
                        denomination = sendingDenomination,
                        dollarText = dollarEquivalentText,
                        onAmountTap = onAmountTap,
                    )
                    HorizontalDivider(color = MaterialTheme.colorScheme.outlineVariant, thickness = 1.dp)
                    AddressWidget(
                        onScanQr = onScanQr,
                        initialAddress = initialAddress,
                        onAddressChanged = onAddressChanged,
                    )
                    HorizontalDivider(color = MaterialTheme.colorScheme.outlineVariant, thickness = 1.dp)
                    SpendingWidget(
                        accountShort = accountShort,
                        feeEta = feeEta,
                        feeAmount = feeAmount,
                        totalSpendingCrypto = totalSpendingCrypto,
                        totalSpendingFiat = totalSpendingFiat,
                        utxoCount = utxoCount,
                        onChangeSpeed = onChangeSpeed,
                        onUtxoDetailsClick = onUtxoDetailsClick,
                    )
                    Spacer(Modifier.weight(1f))
                    ImageButton(
                        text = stringResource(R.string.btn_next),
                        onClick = onNext,
                        colors =
                            ButtonDefaults.buttonColors(
                                containerColor = MaterialTheme.colorScheme.background,
                                contentColor = Color.White,
                            ),
                        modifier =
                            Modifier
                                .fillMaxWidth()
                                .padding(bottom = 24.dp),
                    )
                }
            }
        }
    }

    // Handle sheet presentation
    presenter.sheetState?.let { taggedSheet ->
        when (taggedSheet.item) {
            is org.bitcoinppl.cove.SendFlowPresenter.SheetState.CoinControlCustomAmount -> {
                org.bitcoinppl.cove.send.coin_control.CoinControlCustomAmountSheet(
                    sendFlowManager = sendFlowManager,
                    walletManager = walletManager,
                    utxos = utxos,
                    onDismiss = { presenter.sheetState = null },
                )
            }
            is org.bitcoinppl.cove.SendFlowPresenter.SheetState.Qr -> {
                ModalBottomSheet(
                    onDismissRequest = { presenter.sheetState = null },
                    sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true),
                ) {
                    QrCodeScanView(
                        onScanned = { multiFormat ->
                            presenter.sheetState = null
                            when (multiFormat) {
                                is MultiFormat.Address -> {
                                    sendFlowManager.enteringAddress = multiFormat.v1.address().string()
                                }
                                else -> {
                                    app.alertState =
                                        TaggedItem(
                                            AppAlertState.General(
                                                title = "Invalid QR Code",
                                                message = "Please scan a valid Bitcoin address QR code",
                                            ),
                                        )
                                }
                            }
                        },
                        onDismiss = { presenter.sheetState = null },
                        app = app,
                    )
                }
            }
            is org.bitcoinppl.cove.SendFlowPresenter.SheetState.Fee -> {
                sendFlowManager.feeRateOptions?.let { feeOptions ->
                    sendFlowManager.selectedFeeRate?.let { selectedRate ->
                        FeeRateSelectorSheet(
                            app = app,
                            walletManager = walletManager,
                            sendFlowManager = sendFlowManager,
                            presenter = presenter,
                            feeOptions = feeOptions,
                            selectedOption = selectedRate,
                            onSelectFee = { newFeeOption ->
                                sendFlowManager.dispatch(
                                    SendFlowManagerAction.SelectFeeRate(newFeeOption),
                                )
                            },
                            onUpdateFeeOptions = { newOptions ->
                                sendFlowManager.dispatch(
                                    SendFlowManagerAction.ChangeFeeRateOptions(newOptions),
                                )
                            },
                            onDismiss = { presenter.sheetState = null },
                        )
                    }
                }
            }
        }
    }
}

@Composable
private fun BalanceWidget(
    amount: String,
    denomination: String,
    isHidden: Boolean,
    onToggleVisibility: () -> Unit,
) {
    Box(
        modifier =
            Modifier
                .fillMaxWidth()
                .height(160.dp)
                .padding(horizontal = 16.dp, vertical = 20.dp),
    ) {
        Row(
            modifier =
                Modifier
                    .align(Alignment.BottomStart)
                    .fillMaxWidth(),
        ) {
            Column(modifier = Modifier.weight(1f)) {
                Text(
                    stringResource(R.string.label_balance),
                    color = Color.White.copy(alpha = 0.7f),
                    fontSize = 14.sp,
                )
                Spacer(Modifier.height(4.dp))
                Row(verticalAlignment = Alignment.Bottom) {
                    AutoSizeText(
                        text = if (isHidden) "••••••" else amount,
                        color = Color.White,
                        maxFontSize = 24.sp,
                        minimumScaleFactor = 0.90f,
                        fontWeight = FontWeight.Bold,
                    )
                    Spacer(Modifier.size(6.dp))
                    Text(
                        denomination,
                        color = Color.White,
                        fontSize = 14.sp,
                        modifier = Modifier.offset(y = (-4).dp),
                    )
                }
            }
            IconButton(
                onClick = onToggleVisibility,
                modifier = Modifier.align(Alignment.CenterVertically),
            ) {
                Icon(
                    imageVector = if (isHidden) Icons.Filled.VisibilityOff else Icons.Filled.Visibility,
                    contentDescription = if (isHidden) "Hidden" else "Visible",
                    tint = Color.White,
                    modifier = Modifier.size(24.dp),
                )
            }
        }
    }
}

@Composable
private fun CoinControlAmountWidget(
    amount: String,
    denomination: String,
    dollarText: String,
    onAmountTap: () -> Unit,
) {
    Column(modifier = Modifier.fillMaxWidth()) {
        Spacer(Modifier.height(20.dp))
        Text(
            stringResource(R.string.label_enter_amount),
            color = MaterialTheme.colorScheme.onSurface,
            fontSize = 18.sp,
            fontWeight = FontWeight.SemiBold,
        )
        Spacer(Modifier.height(4.dp))
        Text(
            stringResource(R.string.label_how_much_to_send),
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            fontSize = 14.sp,
        )
        Spacer(Modifier.height(24.dp))
        Row(
            modifier =
                Modifier
                    .fillMaxWidth()
                    .clickable(onClick = onAmountTap),
            verticalAlignment = Alignment.Bottom,
        ) {
            Box(modifier = Modifier.weight(1f), contentAlignment = Alignment.Center) {
                BalanceAutoSizeText(
                    text = amount,
                    color = MaterialTheme.colorScheme.onSurface,
                    baseFontSize = 48.sp,
                    minimumScaleFactor = 0.01f,
                    fontWeight = FontWeight.Bold,
                    textAlign = TextAlign.Right,
                    modifier = Modifier.fillMaxWidth(),
                )
            }
            Spacer(Modifier.width(32.dp))
            Row(verticalAlignment = Alignment.Bottom, modifier = Modifier.offset(y = (-4).dp)) {
                Text(denomination, color = MaterialTheme.colorScheme.onSurface, fontSize = 18.sp, maxLines = 1)
                Spacer(Modifier.width(4.dp))
                Icon(
                    imageVector = Icons.Filled.ArrowDropDown,
                    contentDescription = null,
                    tint = MaterialTheme.colorScheme.onSurface,
                    modifier = Modifier.size(20.dp),
                )
            }
        }
        Spacer(Modifier.height(8.dp))
        Text(
            dollarText,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            fontSize = 16.sp,
            textAlign = TextAlign.Center,
            modifier = Modifier.fillMaxWidth(),
        )
        Spacer(Modifier.height(24.dp))
    }
}

@Composable
private fun AddressWidget(
    onScanQr: () -> Unit,
    initialAddress: String,
    onAddressChanged: (String) -> Unit,
) {
    var address by remember { mutableStateOf(initialAddress) }

    // bidirectional sync: update local state when parent state changes
    androidx.compose.runtime.LaunchedEffect(initialAddress) {
        if (address != initialAddress) {
            address = initialAddress
        }
    }

    Column(modifier = Modifier.fillMaxWidth()) {
        Spacer(Modifier.height(20.dp))
        Row(verticalAlignment = Alignment.CenterVertically) {
            Column(modifier = Modifier.weight(1f)) {
                Text(
                    stringResource(R.string.label_enter_address),
                    color = MaterialTheme.colorScheme.onSurface,
                    fontSize = 18.sp,
                    fontWeight = FontWeight.SemiBold,
                )
                Spacer(Modifier.height(4.dp))
                Text(
                    stringResource(R.string.label_where_send_to),
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    fontSize = 14.sp,
                )
            }
            IconButton(
                onClick = onScanQr,
                modifier = Modifier.offset(x = 8.dp),
            ) {
                Icon(Icons.Filled.QrCode2, contentDescription = null, tint = MaterialTheme.colorScheme.onSurfaceVariant)
            }
        }
        Spacer(Modifier.height(10.dp))
        BasicTextField(
            value = address,
            onValueChange = { newValue ->
                address = newValue
                onAddressChanged(newValue)
            },
            textStyle =
                TextStyle(
                    color = MaterialTheme.colorScheme.onSurface,
                    fontSize = 15.sp,
                    lineHeight = 20.sp,
                    fontWeight = FontWeight.Medium,
                ),
            modifier = Modifier.fillMaxWidth(),
        )
        Spacer(Modifier.height(24.dp))
    }
}

@Composable
private fun SpendingWidget(
    accountShort: String,
    feeEta: String,
    feeAmount: String,
    totalSpendingCrypto: String,
    totalSpendingFiat: String,
    utxoCount: Int,
    onChangeSpeed: () -> Unit,
    onUtxoDetailsClick: () -> Unit,
) {
    Column(modifier = Modifier.fillMaxWidth()) {
        Spacer(Modifier.height(20.dp))
        Row(modifier = Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
            Text(
                stringResource(R.string.label_account),
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                fontSize = 14.sp,
                modifier = Modifier.weight(1f),
            )
            Row(verticalAlignment = Alignment.CenterVertically) {
                Icon(
                    Icons.Filled.CurrencyBitcoin,
                    contentDescription = null,
                    tint = CoveColor.WarningOrange,
                    modifier = Modifier.size(28.dp),
                )
                Spacer(Modifier.size(8.dp))
                Column(horizontalAlignment = Alignment.Start) {
                    Text(accountShort, color = MaterialTheme.colorScheme.onSurfaceVariant, fontSize = 14.sp)
                    Spacer(Modifier.size(4.dp))
                    Text(
                        stringResource(R.string.label_main),
                        color = MaterialTheme.colorScheme.onSurface,
                        fontSize = 14.sp,
                        fontWeight = FontWeight.SemiBold,
                    )
                }
            }
        }
        Spacer(Modifier.height(24.dp))
        Row(modifier = Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
            Column(modifier = Modifier.weight(1f)) {
                Text(
                    stringResource(R.string.label_network_fee),
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    fontSize = 14.sp,
                )
                Spacer(Modifier.height(4.dp))
                Row(verticalAlignment = Alignment.CenterVertically) {
                    Text(feeEta, color = MaterialTheme.colorScheme.onSurfaceVariant, fontSize = 12.sp)
                    Spacer(Modifier.size(4.dp))
                    Text(
                        stringResource(R.string.btn_change_speed),
                        color = CoveColor.LinkBlue,
                        fontSize = 12.sp,
                        modifier =
                            Modifier
                                .clickable(onClick = onChangeSpeed)
                                .padding(4.dp),
                    )
                }
            }
            Text(feeAmount, color = MaterialTheme.colorScheme.onSurfaceVariant, fontSize = 14.sp)
        }
        Spacer(Modifier.height(24.dp))
        Row(modifier = Modifier.fillMaxWidth(), verticalAlignment = Alignment.Top) {
            Text(
                stringResource(R.string.label_total_spending),
                color = MaterialTheme.colorScheme.onSurface,
                fontSize = 14.sp,
                fontWeight = FontWeight.SemiBold,
                modifier = Modifier.weight(1f),
            )
            Column(horizontalAlignment = Alignment.End) {
                Text(
                    totalSpendingCrypto,
                    color = MaterialTheme.colorScheme.onSurface,
                    fontSize = 14.sp,
                    fontWeight = FontWeight.SemiBold,
                )
                Spacer(Modifier.height(8.dp))
                Text(
                    totalSpendingFiat,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    fontSize = 12.sp,
                    textAlign = TextAlign.End,
                )
            }
        }
        Spacer(Modifier.height(8.dp))
        Row(modifier = Modifier.fillMaxWidth()) {
            Text(
                text = if (utxoCount == 1) "Spending 1 UTXO" else "Spending $utxoCount UTXOs",
                color = CoveColor.LinkBlue,
                fontSize = 12.sp,
                modifier =
                    Modifier
                        .clickable(onClick = onUtxoDetailsClick)
                        .padding(4.dp),
            )
            Spacer(Modifier.weight(1f))
        }
        Spacer(Modifier.height(20.dp))
    }
}
