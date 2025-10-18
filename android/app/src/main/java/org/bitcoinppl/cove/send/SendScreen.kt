package org.bitcoinppl.cove.send

import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.verticalScroll
import androidx.compose.foundation.clickable
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.filled.CurrencyBitcoin
import androidx.compose.material.icons.filled.ArrowDropDown
import androidx.compose.material.icons.filled.QrCode2
import androidx.compose.material.icons.filled.Visibility
import androidx.compose.material.icons.filled.VisibilityOff
import androidx.compose.material3.*
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
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
import androidx.compose.ui.tooling.preview.Preview
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.example.cove.R
import org.bitcoinppl.cove.AppManager
import org.bitcoinppl.cove.AppSheetState
import org.bitcoinppl.cove.SendFlowManager
import org.bitcoinppl.cove.SendFlowPresenter
import org.bitcoinppl.cove.TaggedItem
import org.bitcoinppl.cove.WalletManager
import org.bitcoinppl.cove.sheets.FeeRateSelectorSheet
import org.bitcoinppl.cove.ui.theme.CoveColor
import org.bitcoinppl.cove.views.ImageButton

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun SendScreen(
    app: AppManager,
    walletManager: WalletManager,
    sendFlowManager: SendFlowManager,
    presenter: SendFlowPresenter,
    modifier: Modifier = Modifier,
    snackbarHostState: SnackbarHostState = remember { SnackbarHostState() },
) {
    // extract state from managers
    val metadata = walletManager.walletMetadata
    val balance = walletManager.balance.spendable()
    val isBalanceHidden = walletManager.balanceVisibility

    // send flow state
    val amountField = sendFlowManager.amountField
    val addressField = sendFlowManager.enteringAddress
    val selectedFeeRate = sendFlowManager.selectedFeeRate
    val feeOptions = sendFlowManager.feeRateOptions
    val totalSpentBtc = sendFlowManager.totalSpentInBtc
    val totalSpentFiat = sendFlowManager.totalSpentInFiat

    // local state for fee selector sheet
    var showFeeSelector by remember { mutableStateOf(false) }

    // format balance
    val balanceFormatted = when (metadata.selectedUnit) {
        org.bitcoinppl.cove.BitcoinUnit.BTC -> balance.asBtc().toString()
        org.bitcoinppl.cove.BitcoinUnit.SAT -> balance.asSats().toString()
    }
    val balanceDenomination = when (metadata.selectedUnit) {
        org.bitcoinppl.cove.BitcoinUnit.BTC -> "BTC"
        org.bitcoinppl.cove.BitcoinUnit.SAT -> "sats"
    }
    Scaffold(
        containerColor = CoveColor.BackgroundDark,
        topBar = {
            CenterAlignedTopAppBar(
                colors = TopAppBarDefaults.centerAlignedTopAppBarColors(
                    containerColor = Color.Transparent,
                    titleContentColor = Color.White,
                    navigationIconContentColor = Color.White,
                ),
                title = { Text("") },
                navigationIcon = {
                    IconButton(onClick = { app.popRoute() }) {
                        Icon(Icons.AutoMirrored.Filled.ArrowBack, contentDescription = null)
                    }
                }
            )
        },
        snackbarHost = { SnackbarHost(snackbarHostState) },
        modifier = modifier
    ) { padding ->
        Box(
            modifier = Modifier
                .fillMaxSize()
                .padding(padding)
        ) {
            Image(
                painter = painterResource(id = R.drawable.image_chain_code_pattern_horizontal),
                contentDescription = null,
                contentScale = ContentScale.Crop,
                modifier = Modifier
                    .fillMaxHeight()
                    .align(Alignment.TopCenter)
                    .offset(y = (-40).dp)
                    .graphicsLayer(alpha = 0.25f)
            )
            Column(modifier = Modifier.fillMaxSize()) {
                BalanceWidget(
                    amount = balanceFormatted,
                    denomination = balanceDenomination,
                    isHidden = isBalanceHidden,
                    onToggleVisibility = { walletManager.toggleBalanceVisibility() }
                )
                Column(
                    modifier = Modifier
                        .fillMaxWidth()
                        .weight(1f)
                        .clip(RoundedCornerShape(topStart = 0.dp, topEnd = 0.dp))
                        .background(CoveColor.BackgroundLight)
                        .verticalScroll(rememberScrollState())
                        .padding(horizontal = 16.dp)
                ) {
                    AmountWidget(
                        sendFlowManager = sendFlowManager,
                        metadata = metadata
                    )
                    HorizontalDivider(color = CoveColor.DividerLight, thickness = 1.dp)
                    AddressWidget(
                        sendFlowManager = sendFlowManager
                    )
                    HorizontalDivider(color = CoveColor.DividerLight, thickness = 1.dp)

                    // only show spending section if fee rate is calculated
                    selectedFeeRate?.let {
                        SpendingWidget(
                            metadata = metadata,
                            selectedFeeRate = it,
                            totalSpentBtc = totalSpentBtc,
                            totalSpentFiat = totalSpentFiat,
                            onChangeSpeed = {
                                showFeeSelector = true
                            }
                        )
                        Spacer(Modifier.weight(1f))
                        ImageButton(
                            text = stringResource(R.string.btn_next),
                            onClick = {
                                // validate and go to next screen
                                if (sendFlowManager.validate(displayAlert = true)) {
                                    sendFlowManager.dispatch(org.bitcoinppl.cove.SendFlowAction.FinalizeAndGoToNextScreen)
                                }
                            },
                            colors = ButtonDefaults.buttonColors(
                                containerColor = CoveColor.BackgroundDark,
                                contentColor = Color.White
                            ),
                            modifier = Modifier
                                .fillMaxWidth()
                                .padding(bottom = 24.dp)
                        )
                    }
                }
            }
        }
    }

    // fee selector sheet
    if (showFeeSelector && selectedFeeRate != null && feeOptions != null) {
        FeeRateSelectorSheet(
            app = app,
            manager = walletManager,
            presenter = presenter,
            feeOptions = feeOptions,
            selectedOption = selectedFeeRate,
            onSelectFee = { newFeeRate ->
                sendFlowManager.dispatch(
                    org.bitcoinppl.cove.SendFlowAction.ChangeFeeRate(newFeeRate)
                )
                showFeeSelector = false
            },
            onDismiss = {
                showFeeSelector = false
            }
        )
    }

    // send flow presenter alerts
    presenter.alertState?.let { taggedAlert ->
        AlertDialog(
            onDismissRequest = { presenter.alertState = null },
            title = { Text(presenter.alertTitle()) },
            text = { Text(presenter.alertMessage()) },
            confirmButton = {
                TextButton(
                    onClick = {
                        presenter.alertButtonAction()?.invoke()
                    }
                ) {
                    Text("OK")
                }
            }
        )
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
        modifier = Modifier
            .fillMaxWidth()
            .height(160.dp)
            .padding(horizontal = 16.dp, vertical = 20.dp)
    ) {
        Row(
            modifier = Modifier
                .align(Alignment.BottomStart)
                .fillMaxWidth(),
        ) {
            Column(modifier = Modifier.weight(1f)) {
                Text(
                    stringResource(R.string.label_balance),
                    color = Color.White.copy(alpha = 0.7f),
                    fontSize = 14.sp
                )
                Spacer(Modifier.height(4.dp))
                Row(verticalAlignment = Alignment.Bottom) {
                    Text(
                        text = if (isHidden) "••••••" else amount,
                        color = Color.White,
                        fontSize = 24.sp,
                        fontWeight = FontWeight.Bold,
                    )
                    Spacer(Modifier.size(6.dp))
                    Text(
                        denomination,
                        color = Color.White,
                        fontSize = 14.sp,
                        modifier = Modifier.offset(y = (-4).dp)
                    )
                }
            }
            IconButton(
                onClick = onToggleVisibility,
                modifier = Modifier.offset(y = 8.dp, x = 8.dp)
            ) {
                Icon(
                    imageVector = if (isHidden) Icons.Filled.VisibilityOff else Icons.Filled.Visibility,
                    contentDescription = null,
                    tint = Color.White
                )
            }
        }
    }
}

@Composable
private fun AmountWidget(
    sendFlowManager: SendFlowManager,
    metadata: org.bitcoinppl.cove.WalletMetadata
) {
    val amountField = sendFlowManager.amountField
    val amountInFiat = sendFlowManager.sendAmountFiat

    val denomination = when (metadata.selectedUnit) {
        org.bitcoinppl.cove.BitcoinUnit.BTC -> "BTC"
        org.bitcoinppl.cove.BitcoinUnit.SAT -> "sats"
    }

    Column(modifier = Modifier.fillMaxWidth()) {
        Spacer(Modifier.height(20.dp))
        Text(
            stringResource(R.string.label_enter_amount),
            color = CoveColor.TextPrimary,
            fontSize = 18.sp,
            fontWeight = FontWeight.SemiBold
        )
        Spacer(Modifier.height(4.dp))
        Text(
            stringResource(R.string.label_how_much_to_send),
            color = CoveColor.TextSecondary,
            fontSize = 14.sp
        )
        Spacer(Modifier.height(24.dp))
        Row(
            modifier = Modifier.fillMaxWidth(),
            verticalAlignment = Alignment.Bottom
        ) {
            Box(modifier = Modifier.weight(1f), contentAlignment = Alignment.Center) {
                BasicTextField(
                    value = amountField,
                    onValueChange = { newValue ->
                        sendFlowManager.dispatch(org.bitcoinppl.cove.SendFlowAction.ChangeAmountField(newValue))
                    },
                    textStyle = TextStyle(
                        color = CoveColor.TextPrimary,
                        fontSize = 48.sp,
                        fontWeight = FontWeight.Bold,
                        textAlign = TextAlign.Right
                    ),
                    singleLine = true,
                    modifier = Modifier.fillMaxWidth()
                )
            }
            Spacer(Modifier.width(32.dp))
            Row(verticalAlignment = Alignment.Bottom, modifier = Modifier.offset(y = (-4).dp)) {
                Text(denomination, color = CoveColor.TextPrimary, fontSize = 18.sp, maxLines = 1)
                Spacer(Modifier.width(4.dp))
                Icon(
                    imageVector = Icons.Filled.ArrowDropDown,
                    contentDescription = null,
                    tint = CoveColor.TextPrimary,
                    modifier = Modifier.size(20.dp)
                )
            }
        }
        Spacer(Modifier.height(8.dp))
        Text(
            amountInFiat,
            color = CoveColor.TextSecondary,
            fontSize = 16.sp,
            textAlign = TextAlign.Center,
            modifier = Modifier.fillMaxWidth()
        )
        Spacer(Modifier.height(24.dp))
    }
}

@Composable
private fun AddressWidget(
    sendFlowManager: SendFlowManager
) {
    val addressField = sendFlowManager.enteringAddress

    Column(modifier = Modifier.fillMaxWidth()) {
        Spacer(Modifier.height(20.dp))
        Row(verticalAlignment = Alignment.CenterVertically) {
            Column(modifier = Modifier.weight(1f)) {
                Text(
                    stringResource(R.string.label_enter_address),
                    color = CoveColor.TextPrimary,
                    fontSize = 18.sp,
                    fontWeight = FontWeight.SemiBold,
                )
                Spacer(Modifier.height(4.dp))
                Text(
                    stringResource(R.string.label_where_send_to),
                    color = CoveColor.TextSecondary,
                    fontSize = 14.sp
                )
            }
            IconButton(
                onClick = {
                    app.sheetState = TaggedItem(AppSheetState.Qr)
                },
                modifier = Modifier.offset(x = 8.dp)

            ) {
                Icon(Icons.Filled.QrCode2, contentDescription = null, tint = CoveColor.IconGray)
            }
        }
        Spacer(Modifier.height(10.dp))
        BasicTextField(
            value = addressField,
            onValueChange = { newValue ->
                sendFlowManager.dispatch(org.bitcoinppl.cove.SendFlowAction.ChangeEnteringAddress(newValue))
            },
            textStyle = TextStyle(
                color = CoveColor.TextPrimary,
                fontSize = 15.sp,
                lineHeight = 20.sp,
                fontWeight = FontWeight.Medium,
            ),
            modifier = Modifier.fillMaxWidth()
        )
        Spacer(Modifier.height(24.dp))
    }
}

@Composable
private fun SpendingWidget(
    metadata: org.bitcoinppl.cove.WalletMetadata,
    selectedFeeRate: org.bitcoinppl.cove.FeeRateOptionWithTotalFee,
    totalSpentBtc: String,
    totalSpentFiat: String,
    onChangeSpeed: () -> Unit,
) {
    // format account identifier (fingerprint or ident)
    val accountShort = metadata.identOrFingerprint()

    Column(modifier = Modifier.fillMaxWidth()) {
        Spacer(Modifier.height(20.dp))
        Row(modifier = Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
            Text(
                stringResource(R.string.label_account),
                color = CoveColor.TextSecondary,
                fontSize = 14.sp,
                modifier = Modifier.weight(1f)
            )
            Row(verticalAlignment = Alignment.CenterVertically) {
                Icon(
                    Icons.Filled.CurrencyBitcoin,
                    contentDescription = null,
                    tint = CoveColor.WarningOrange,
                    modifier = Modifier.size(28.dp)
                )
                Spacer(Modifier.size(8.dp))
                Column(horizontalAlignment = Alignment.Start) {
                    Text(accountShort, color = CoveColor.TextSecondary, fontSize = 14.sp)
                    Spacer(Modifier.size(4.dp))
                    Text(
                        stringResource(R.string.label_main),
                        color = CoveColor.TextPrimary,
                        fontSize = 14.sp,
                        fontWeight = FontWeight.SemiBold
                    )
                }
            }
        }
        Spacer(Modifier.height(24.dp))
        Row(modifier = Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
            Column(modifier = Modifier.weight(1f)) {
                Text(
                    stringResource(R.string.label_network_fee),
                    color = CoveColor.TextSecondary,
                    fontSize = 14.sp
                )
                Spacer(Modifier.height(4.dp))
                Row(verticalAlignment = Alignment.CenterVertically) {
                    Text(selectedFeeRate.duration(), color = CoveColor.TextSecondary, fontSize = 12.sp)
                    Spacer(Modifier.size(4.dp))
                    Text(
                        stringResource(R.string.btn_change_speed),
                        color = CoveColor.LinkBlue,
                        fontSize = 12.sp,
                        modifier = Modifier
                            .clickable(onClick = onChangeSpeed)
                            .padding(4.dp)
                    )
                }
            }
            Text(selectedFeeRate.totalFeeString(), color = CoveColor.TextSecondary, fontSize = 14.sp)
        }
        Spacer(Modifier.height(24.dp))
        Row(modifier = Modifier.fillMaxWidth(), verticalAlignment = Alignment.Top) {
            Text(
                stringResource(R.string.label_total_spending),
                color = CoveColor.TextPrimary,
                fontSize = 14.sp,
                fontWeight = FontWeight.SemiBold,
                modifier = Modifier.weight(1f)
            )
            Column(horizontalAlignment = Alignment.End) {
                Text(
                    totalSpentBtc,
                    color = CoveColor.TextPrimary,
                    fontSize = 14.sp,
                    fontWeight = FontWeight.SemiBold
                )
                Spacer(Modifier.height(8.dp))
                Text(
                    totalSpentFiat,
                    color = CoveColor.TextSecondary,
                    fontSize = 12.sp,
                    textAlign = TextAlign.End
                )
            }
        }
        Spacer(Modifier.height(20.dp))
    }
}
