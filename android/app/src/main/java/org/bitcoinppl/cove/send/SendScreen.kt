package org.bitcoinppl.cove.send

import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.consumeWindowInsets
import androidx.compose.foundation.layout.ime
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.navigationBars
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.filled.ArrowDropDown
import androidx.compose.material.icons.filled.Cancel
import androidx.compose.material.icons.filled.Clear
import androidx.compose.material.icons.filled.CurrencyBitcoin
import androidx.compose.material.icons.filled.QrCode2
import androidx.compose.material.icons.filled.Visibility
import androidx.compose.material.icons.filled.VisibilityOff
import androidx.compose.material3.*
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.focus.onFocusChanged
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalConfiguration
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.LocalFocusManager
import androidx.compose.ui.platform.LocalSoftwareKeyboardController
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import org.bitcoinppl.cove.AppManager
import org.bitcoinppl.cove.R
import org.bitcoinppl.cove.SendFlowManager
import org.bitcoinppl.cove.SendFlowPresenter
import org.bitcoinppl.cove.WalletManager
import org.bitcoinppl.cove.ui.theme.CoveColor
import org.bitcoinppl.cove.ui.theme.ForceLightStatusBarIcons
import org.bitcoinppl.cove.ui.theme.coveColors
import org.bitcoinppl.cove.views.AsyncText
import org.bitcoinppl.cove.views.AutoSizeText
import org.bitcoinppl.cove.views.AutoSizeTextField
import org.bitcoinppl.cove_core.FiatOrBtc
import org.bitcoinppl.cove_core.SendFlowManagerAction
import org.bitcoinppl.cove_core.SetAmountFocusField
import org.bitcoinppl.cove_core.WalletManagerAction
import org.bitcoinppl.cove_core.types.BitcoinUnit
import org.bitcoinppl.cove_core.types.FeeSpeed
import org.bitcoinppl.cove_core.types.addressStringSpacedOut

private enum class SendFocusField { None, Amount, Address }

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun SendScreen(
    app: AppManager,
    walletManager: WalletManager,
    sendFlowManager: SendFlowManager,
    presenter: SendFlowPresenter,
    snackbarHostState: SnackbarHostState = remember { SnackbarHostState() },
    onBack: () -> Unit,
    onNext: () -> Unit,
    onScanQr: () -> Unit,
    onChangeSpeed: () -> Unit,
) {
    var focusedField by remember { mutableStateOf(SendFocusField.None) }
    val focusManager = LocalFocusManager.current
    val keyboardController = LocalSoftwareKeyboardController.current
    val addressFocusRequester = remember { FocusRequester() }
    val amountFocusRequester = remember { FocusRequester() }

    // derive state from managers (like iOS @Environment)
    val metadata = walletManager.walletMetadata
    val isFiatMode = metadata?.fiatOrBtc == FiatOrBtc.FIAT
    val isBalanceHidden = !(metadata?.sensitiveVisible ?: true)
    val balanceAmount = walletManager.amountFmt(walletManager.balance.spendable())
    val balanceDenomination = walletManager.unit
    val amountText =
        when (metadata?.fiatOrBtc) {
            FiatOrBtc.BTC -> sendFlowManager.enteringBtcAmount
            FiatOrBtc.FIAT -> sendFlowManager.enteringFiatAmount
            else -> sendFlowManager.enteringBtcAmount
        }
    val amountDenomination =
        when (metadata?.fiatOrBtc) {
            FiatOrBtc.BTC -> walletManager.unit
            FiatOrBtc.FIAT -> ""
            else -> walletManager.unit
        }
    val dollarEquivalentText =
        when (metadata?.fiatOrBtc) {
            FiatOrBtc.FIAT -> sendFlowManager.sendAmountBtc
            else -> sendFlowManager.sendAmountFiat
        }
    val secondaryUnit =
        when (metadata?.fiatOrBtc) {
            FiatOrBtc.FIAT -> walletManager.unit
            else -> ""
        }
    val initialAddress = sendFlowManager.enteringAddress
    val accountShort = metadata?.masterFingerprint?.asUppercase()?.take(8) ?: ""
    val feeEta =
        sendFlowManager.selectedFeeRate?.let {
            when (it.feeSpeed()) {
                is FeeSpeed.Slow -> "~1 hour"
                is FeeSpeed.Medium -> "~30 minutes"
                is FeeSpeed.Fast -> "~10 minutes"
                is FeeSpeed.Custom -> "Custom"
            }
        } ?: "~30 minutes"
    val feeAmount = sendFlowManager.totalFeeString
    val totalSpendingCrypto = sendFlowManager.totalSpentInBtc
    val totalSpendingFiat = sendFlowManager.totalSpentInFiat
    val exceedsBalance = sendFlowManager.rust.amountExceedsBalance()

    // bidirectional sync: observe presenter.focusField and update UI focus
    LaunchedEffect(presenter.focusField) {
        when (presenter.focusField) {
            SetAmountFocusField.AMOUNT -> {
                kotlinx.coroutines.delay(350)
                amountFocusRequester.requestFocus()
                keyboardController?.show()
            }
            SetAmountFocusField.ADDRESS -> {
                kotlinx.coroutines.delay(350)
                addressFocusRequester.requestFocus()
                keyboardController?.show()
            }
            null -> focusManager.clearFocus()
        }
    }

    // force white status bar icons for midnight blue background
    ForceLightStatusBarIcons()

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
                val configuration = LocalConfiguration.current
                val screenHeightDp = configuration.screenHeightDp.dp
                val headerHeight = screenHeightDp * 0.145f

                BalanceWidget(
                    amount = balanceAmount,
                    denomination = balanceDenomination,
                    isHidden = isBalanceHidden,
                    onToggleVisibility = {
                        walletManager.dispatch(WalletManagerAction.ToggleSensitiveVisibility)
                    },
                    height = headerHeight,
                )

                val density = LocalDensity.current
                val isKeyboardVisible = WindowInsets.ime.getBottom(density) > 0

                Box(
                    modifier =
                        Modifier
                            .fillMaxWidth()
                            .weight(1f)
                            .clip(RoundedCornerShape(topStart = 0.dp, topEnd = 0.dp))
                            .background(MaterialTheme.colorScheme.surface),
                ) {
                    Column(
                        modifier =
                            Modifier
                                .fillMaxSize()
                                .verticalScroll(rememberScrollState())
                                .padding(horizontal = 16.dp),
                    ) {
                        AmountWidget(
                            initialAmount = amountText,
                            denomination = amountDenomination,
                            dollarText = dollarEquivalentText,
                            secondaryUnit = secondaryUnit,
                            onAmountChanged = { newAmount ->
                                when (metadata?.fiatOrBtc) {
                                    FiatOrBtc.BTC -> sendFlowManager.updateEnteringBtcAmount(newAmount)
                                    FiatOrBtc.FIAT -> sendFlowManager.updateEnteringFiatAmount(newAmount)
                                    else -> sendFlowManager.updateEnteringBtcAmount(newAmount)
                                }
                            },
                            onClearAmount = {
                                sendFlowManager.dispatch(SendFlowManagerAction.ClearSendAmount)
                            },
                            onUnitChange = { unit ->
                                val bitcoinUnit =
                                    when (unit.lowercase()) {
                                        "sats" -> BitcoinUnit.SAT
                                        "btc" -> BitcoinUnit.BTC
                                        else -> BitcoinUnit.SAT
                                    }
                                walletManager.dispatch(WalletManagerAction.UpdateUnit(bitcoinUnit))
                            },
                            onToggleFiatOrBtc = {
                                walletManager.dispatch(WalletManagerAction.ToggleFiatOrBtc)
                            },
                            onSanitizeBtcAmount = { oldValue, newValue ->
                                sendFlowManager.rust.sanitizeBtcEnteringAmount(oldValue, newValue)
                            },
                            onSanitizeFiatAmount = { oldValue, newValue ->
                                sendFlowManager.rust.sanitizeFiatEnteringAmount(oldValue, newValue)
                            },
                            isFiatMode = isFiatMode,
                            exceedsBalance = exceedsBalance,
                            focusRequester = amountFocusRequester,
                            onFocusChanged = { focused ->
                                focusedField = if (focused) SendFocusField.Amount else SendFocusField.None
                                presenter.focusField = if (focused) SetAmountFocusField.AMOUNT else null
                            },
                            onDone = {
                                presenter.focusField =
                                    if (!sendFlowManager.rust.validateAddress()) {
                                        SetAmountFocusField.ADDRESS
                                    } else {
                                        null
                                    }
                            },
                        )
                        HorizontalDivider(color = MaterialTheme.colorScheme.outlineVariant, thickness = 1.dp)
                        AddressWidget(
                            onScanQr = onScanQr,
                            initialAddress = initialAddress,
                            onAddressChanged = { newAddress ->
                                sendFlowManager.enteringAddress = newAddress
                            },
                            focusRequester = addressFocusRequester,
                            onFocusChanged = { focused ->
                                focusedField = if (focused) SendFocusField.Address else SendFocusField.None
                                presenter.focusField = if (focused) SetAmountFocusField.ADDRESS else null
                            },
                            onDone = {
                                presenter.focusField =
                                    if (!sendFlowManager.rust.validateAmount()) {
                                        SetAmountFocusField.AMOUNT
                                    } else {
                                        null
                                    }
                            },
                        )
                        HorizontalDivider(color = MaterialTheme.colorScheme.outlineVariant, thickness = 1.dp)
                        SpendingWidget(
                            accountShort = accountShort,
                            feeEta = feeEta,
                            feeAmount = feeAmount,
                            totalSpendingCrypto = totalSpendingCrypto,
                            totalSpendingFiat = totalSpendingFiat,
                            onChangeSpeed = onChangeSpeed,
                        )
                        Spacer(Modifier.weight(1f))
                        ImageButton(
                            text = stringResource(R.string.btn_next),
                            onClick = onNext,
                            colors =
                                ButtonDefaults.buttonColors(
                                    containerColor = MaterialTheme.coveColors.midnightBtn,
                                    contentColor = Color.White,
                                ),
                            modifier =
                                Modifier
                                    .fillMaxWidth()
                                    .padding(bottom = 24.dp),
                        )
                    }

                    // Keyboard toolbar - only show for amount field
                    if (isKeyboardVisible && focusedField == SendFocusField.Amount) {
                        KeyboardToolbar(
                            onMaxSelected = {
                                sendFlowManager.dispatch(SendFlowManagerAction.SelectMaxSend)
                            },
                            onNextOrDone = {
                                if (initialAddress.isEmpty()) {
                                    addressFocusRequester.requestFocus()
                                } else {
                                    focusManager.clearFocus()
                                }
                            },
                            onClear = {
                                sendFlowManager.dispatch(SendFlowManagerAction.ClearSendAmount)
                            },
                            hasAddress = initialAddress.isNotEmpty(),
                            modifier =
                                Modifier
                                    .align(Alignment.BottomCenter)
                                    .consumeWindowInsets(WindowInsets.navigationBars)
                                    .imePadding(),
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
    height: androidx.compose.ui.unit.Dp = 160.dp,
) {
    Box(
        modifier =
            Modifier
                .fillMaxWidth()
                .height(height)
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
private fun AmountWidget(
    initialAmount: String,
    denomination: String,
    dollarText: String,
    secondaryUnit: String = "",
    onAmountChanged: (String) -> Unit,
    onClearAmount: () -> Unit = {},
    onUnitChange: (String) -> Unit = {},
    onToggleFiatOrBtc: () -> Unit = {},
    onSanitizeBtcAmount: (oldValue: String, newValue: String) -> String? = { _, _ -> null },
    onSanitizeFiatAmount: (oldValue: String, newValue: String) -> String? = { _, _ -> null },
    isFiatMode: Boolean = false,
    exceedsBalance: Boolean = false,
    focusRequester: FocusRequester? = null,
    onFocusChanged: (Boolean) -> Unit = {},
    onDone: () -> Unit = {},
) {
    var amount by remember { mutableStateOf(initialAmount) }
    var showUnitMenu by remember { mutableStateOf(false) }
    var textWidth by remember { mutableStateOf(0.dp) }
    var isFocused by remember { mutableStateOf(false) }

    // offset to compensate for unit dropdown (matches iOS)
    val configuration = LocalConfiguration.current
    val screenWidthDp = configuration.screenWidthDp.dp
    val amountOffset =
        if (isFiatMode) {
            0.dp
        } else {
            if (denomination.lowercase() == "btc") screenWidthDp * 0.10f else screenWidthDp * 0.11f
        }

    // bidirectional sync: update local state when parent state changes
    LaunchedEffect(initialAmount) {
        if (amount != initialAmount) {
            amount = initialAmount
        }
    }

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
            modifier = Modifier.fillMaxWidth(),
            verticalAlignment = Alignment.Bottom,
        ) {
            Box(modifier = Modifier.weight(1f), contentAlignment = Alignment.Center) {
                AutoSizeTextField(
                    value = amount,
                    onValueChange = { newValue ->
                        val oldValue = amount
                        // sanitize synchronously before updating local state (matches iOS pattern)
                        val sanitized =
                            if (isFiatMode) {
                                onSanitizeFiatAmount(oldValue, newValue) ?: newValue
                            } else {
                                onSanitizeBtcAmount(oldValue, newValue) ?: newValue
                            }
                        // only update if changed
                        if (sanitized != oldValue) {
                            amount = sanitized
                            onAmountChanged(sanitized)
                        }
                    },
                    maxFontSize = 48.sp,
                    minimumScaleFactor = 0.01f,
                    color = if (exceedsBalance) CoveColor.WarningOrange else MaterialTheme.colorScheme.onSurface,
                    fontWeight = FontWeight.Bold,
                    textAlign = TextAlign.Center,
                    modifier = Modifier.fillMaxWidth().offset(x = amountOffset),
                    onTextWidthChanged = { width -> textWidth = width },
                    onFocusChanged = { focused ->
                        isFocused = focused
                        onFocusChanged(focused)
                    },
                    keyboardActions = KeyboardActions(onDone = { onDone() }),
                    focusRequester = focusRequester,
                )
            }
            // unit dropdown area (only shown when in BTC mode, matches iOS)
            if (!isFiatMode) {
                Spacer(Modifier.width(32.dp))
                Box {
                    Row(
                        verticalAlignment = Alignment.Bottom,
                        modifier =
                            Modifier
                                .offset(y = (-4).dp)
                                .clickable { showUnitMenu = true },
                    ) {
                        Text(denomination, color = MaterialTheme.colorScheme.onSurface, fontSize = 18.sp, maxLines = 1)
                        Spacer(Modifier.width(4.dp))
                        Icon(
                            imageVector = Icons.Filled.ArrowDropDown,
                            contentDescription = null,
                            tint = MaterialTheme.colorScheme.onSurface,
                            modifier = Modifier.size(20.dp),
                        )
                    }
                    DropdownMenu(
                        expanded = showUnitMenu,
                        onDismissRequest = { showUnitMenu = false },
                    ) {
                        DropdownMenuItem(
                            text = { Text("sats") },
                            onClick = {
                                onUnitChange("sats")
                                showUnitMenu = false
                            },
                        )
                        DropdownMenuItem(
                            text = { Text("btc") },
                            onClick = {
                                onUnitChange("btc")
                                showUnitMenu = false
                            },
                        )
                    }
                }
            }
        }
        Spacer(Modifier.height(8.dp))
        Row(
            modifier = Modifier.fillMaxWidth(),
            horizontalArrangement = Arrangement.Center,
        ) {
            Row(
                modifier =
                    Modifier
                        .clickable(onClick = onToggleFiatOrBtc)
                        .padding(vertical = 8.dp)
                        .then(
                            // add horizontal padding in fiat mode (no dropdown to conflict with)
                            if (isFiatMode) Modifier.padding(horizontal = 24.dp) else Modifier,
                        ),
                horizontalArrangement = Arrangement.Center,
            ) {
                Text(
                    dollarText,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    fontSize = 16.sp,
                )
                if (isFiatMode && secondaryUnit.isNotEmpty()) {
                    Spacer(Modifier.width(4.dp))
                    Text(
                        secondaryUnit,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                        fontSize = 16.sp,
                    )
                }
            }
        }
        Spacer(Modifier.height(24.dp))
    }
}

@Composable
private fun AddressWidget(
    onScanQr: () -> Unit,
    initialAddress: String,
    onAddressChanged: (String) -> Unit,
    focusRequester: FocusRequester,
    onFocusChanged: (Boolean) -> Unit = {},
    onDone: () -> Unit = {},
) {
    var address by remember { mutableStateOf(initialAddress) }
    var isFocused by remember { mutableStateOf(false) }

    // bidirectional sync: update local state when parent state changes
    LaunchedEffect(initialAddress) {
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
            // clear button - only visible when focused and has content
            if (isFocused && address.isNotEmpty()) {
                IconButton(
                    onClick = {
                        address = ""
                        onAddressChanged("")
                    },
                    modifier = Modifier.size(32.dp),
                ) {
                    Icon(
                        imageVector = Icons.Filled.Clear,
                        contentDescription = "Clear address",
                        tint = MaterialTheme.colorScheme.onSurfaceVariant,
                        modifier = Modifier.size(20.dp),
                    )
                }
            }
            IconButton(
                onClick = onScanQr,
                modifier = Modifier.offset(x = 8.dp),
            ) {
                Icon(Icons.Filled.QrCode2, contentDescription = null, tint = MaterialTheme.colorScheme.onSurfaceVariant)
            }
        }
        Spacer(Modifier.height(10.dp))
        Box(modifier = Modifier.fillMaxWidth()) {
            BasicTextField(
                value = if (isFocused) address else "",
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
                keyboardOptions = KeyboardOptions(imeAction = ImeAction.Done),
                keyboardActions = KeyboardActions(onDone = { onDone() }),
                modifier =
                    Modifier
                        .fillMaxWidth()
                        .focusRequester(focusRequester)
                        .onFocusChanged { focusState ->
                            isFocused = focusState.isFocused
                            onFocusChanged(focusState.isFocused)
                        },
            )
            // show spaced-out address when not focused
            if (!isFocused && address.isNotEmpty()) {
                Text(
                    text = addressStringSpacedOut(address),
                    color = MaterialTheme.colorScheme.onSurface,
                    fontSize = 15.sp,
                    lineHeight = 20.sp,
                    fontWeight = FontWeight.Medium,
                    maxLines = 3,
                    modifier =
                        Modifier
                            .fillMaxWidth()
                            .clickable { focusRequester.requestFocus() },
                )
            }
            // placeholder when empty and not focused
            if (address.isEmpty() && !isFocused) {
                Text(
                    text = "bc1p...",
                    color = MaterialTheme.colorScheme.onSurfaceVariant.copy(alpha = 0.6f),
                    fontSize = 15.sp,
                    lineHeight = 20.sp,
                    fontWeight = FontWeight.Medium,
                )
            }
        }
        Spacer(Modifier.height(24.dp))
    }
}

@Composable
private fun SpendingWidget(
    accountShort: String,
    feeEta: String,
    feeAmount: String?,
    totalSpendingCrypto: String,
    totalSpendingFiat: String,
    onChangeSpeed: () -> Unit,
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
            AsyncText(text = feeAmount, color = MaterialTheme.colorScheme.onSurfaceVariant, style = MaterialTheme.typography.bodyMedium)
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
        Spacer(Modifier.height(20.dp))
    }
}

@Composable
private fun KeyboardToolbar(
    onMaxSelected: () -> Unit,
    onNextOrDone: () -> Unit,
    onClear: () -> Unit,
    hasAddress: Boolean,
    modifier: Modifier = Modifier,
) {
    val buttonText = if (!hasAddress) "Next" else "Done"

    Surface(
        modifier = modifier.fillMaxWidth(),
        color = MaterialTheme.colorScheme.surface,
    ) {
        Row(
            modifier =
                Modifier
                    .fillMaxWidth()
                    .padding(horizontal = 8.dp, vertical = 4.dp),
            horizontalArrangement = Arrangement.SpaceBetween,
            verticalAlignment = Alignment.CenterVertically,
        ) {
            // Left: Next or Done
            FilledTonalButton(onClick = onNextOrDone) {
                Text(buttonText)
            }

            // Right: Max + Clear
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                FilledTonalButton(onClick = onMaxSelected) {
                    Text("Max")
                }

                FilledTonalButton(onClick = onClear) {
                    Icon(
                        Icons.Filled.Cancel,
                        contentDescription = "Clear",
                        modifier = Modifier.size(20.dp),
                    )
                }
            }
        }
    }
}

@Composable
private fun ImageButton(
    text: String,
    onClick: () -> Unit,
    colors: ButtonColors,
    modifier: Modifier = Modifier,
) {
    Button(
        onClick = onClick,
        colors = colors,
        modifier = modifier,
        shape = RoundedCornerShape(10.dp),
    ) {
        Text(
            text = text,
            fontWeight = FontWeight.SemiBold,
            modifier = Modifier.padding(vertical = 8.dp),
        )
    }
}
