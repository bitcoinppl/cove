package org.bitcoinppl.cove.flows.SendFlow

import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.consumeWindowInsets
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.ime
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.navigationBars
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.filled.Cancel
import androidx.compose.material.icons.filled.CurrencyBitcoin
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonColors
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.CenterAlignedTopAppBar
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.FilledTonalButton
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Scaffold
import androidx.compose.material3.SnackbarHost
import androidx.compose.material3.SnackbarHostState
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TopAppBarDefaults
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
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalConfiguration
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.LocalFocusManager
import androidx.compose.ui.platform.LocalSoftwareKeyboardController
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import org.bitcoinppl.cove.AppManager
import org.bitcoinppl.cove.R
import org.bitcoinppl.cove.WalletManager
import org.bitcoinppl.cove.ui.theme.CoveColor
import org.bitcoinppl.cove.ui.theme.ForceLightStatusBarIcons
import org.bitcoinppl.cove.ui.theme.coveColors
import org.bitcoinppl.cove.views.AsyncText
import org.bitcoinppl.cove_core.FiatOrBtc
import org.bitcoinppl.cove_core.SendFlowManagerAction
import org.bitcoinppl.cove_core.SetAmountFocusField
import org.bitcoinppl.cove_core.WalletManagerAction
import org.bitcoinppl.cove_core.types.BitcoinUnit
import org.bitcoinppl.cove_core.types.FeeSpeed

private enum class SendFocusField { None, Amount, Address }

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun SendFlowSetAmountScreen(
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
    val isBalanceHidden = !(metadata?.sensitiveVisible ?: false)
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

    // initial focus logic: set focus based on validation state (matches iOS behavior)
    LaunchedEffect(sendFlowManager) {
        val amount = sendFlowManager.rust.amount()
        val isAmountInvalid = amount.asSats() == 0uL
        val isAddressEmpty = initialAddress.isEmpty()

        presenter.focusField =
            when {
                isAmountInvalid -> SetAmountFocusField.AMOUNT
                isAddressEmpty -> SetAmountFocusField.ADDRESS
                else -> null
            }
    }

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

                SendFlowHeaderView(
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
                        EnterAmountView(
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
                        EnterAddressView(
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
