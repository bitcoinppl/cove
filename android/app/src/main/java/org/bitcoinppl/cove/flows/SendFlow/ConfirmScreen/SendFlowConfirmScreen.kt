@file:Suppress("PackageNaming")

package org.bitcoinppl.cove.flows.SendFlow.ConfirmScreen

import androidx.compose.animation.core.Animatable
import androidx.compose.animation.core.RepeatMode
import androidx.compose.animation.core.animateFloat
import androidx.compose.animation.core.infiniteRepeatable
import androidx.compose.animation.core.rememberInfiniteTransition
import androidx.compose.animation.core.tween
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.gestures.Orientation
import androidx.compose.foundation.gestures.draggable
import androidx.compose.foundation.gestures.rememberDraggableState
import androidx.compose.foundation.selection.toggleable
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.automirrored.filled.ArrowForward
import androidx.compose.material.icons.filled.ArrowDropDown
import androidx.compose.material.icons.filled.Check
import androidx.compose.material.icons.filled.Close
import androidx.compose.material.icons.filled.Visibility
import androidx.compose.material.icons.filled.VisibilityOff
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.layout.onGloballyPositioned
import androidx.compose.ui.layout.onSizeChanged
import androidx.compose.ui.platform.LocalConfiguration
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import kotlinx.coroutines.launch
import org.bitcoinppl.cove.AppManager
import org.bitcoinppl.cove.R
import org.bitcoinppl.cove.WalletManager
import org.bitcoinppl.cove.flows.SendFlow.Common.SendFlowAccountSection
import org.bitcoinppl.cove.flows.SendFlow.Common.PayjoinWaitingCard
import org.bitcoinppl.cove.flows.SendFlow.Common.SendFlowAdvancedDetailsView
import org.bitcoinppl.cove.flows.SendFlow.SendState
import org.bitcoinppl.cove.ui.theme.CoveColor
import org.bitcoinppl.cove.ui.theme.ForceLightStatusBarIcons
import org.bitcoinppl.cove.ui.theme.coveColors
import org.bitcoinppl.cove.views.AddressText
import org.bitcoinppl.cove.views.AutoSizeText
import org.bitcoinppl.cove.views.KeyValueRow
import org.bitcoinppl.cove.views.lockaware.ModalBottomSheet
import org.bitcoinppl.cove.views.FlexibleSpacer
import org.bitcoinppl.cove_core.PayjoinIntent
import org.bitcoinppl.cove_core.PayjoinSessionId
import org.bitcoinppl.cove_core.PayjoinStatus
import org.bitcoinppl.cove_core.WalletManagerAction
import org.bitcoinppl.cove_core.WalletMetadata
import org.bitcoinppl.cove_core.types.ConfirmDetails

private enum class SheetState {
    AdvancedDetails,
}

// Animation duration in milliseconds for swipe button returning to start position when swipe is incomplete.
private const val SWIPE_RETURN_DURATION_MS = 500

// Progress threshold (0.0-1.0) at which swipe is considered complete and triggers the action.
private const val SWIPE_COMPLETE_THRESHOLD = 0.9f

// Target text color (white) that text animates to during swipe gesture.
private val SWIPE_BUTTON_TEXT_COLOR_TARGET = CoveColor.SwipeButtonText

internal const val SEND_CONFIRM_PAYJOIN_TOGGLE_TAG = "sendConfirm.payjoinToggle"
internal const val SEND_CONFIRM_PAYJOIN_FEE_HELP_TAG = "sendConfirm.payjoinFeeHelp"
internal const val SEND_CONFIRM_TOTAL_SPENDING_TAG = "sendConfirm.totalSpending"
internal const val SEND_CONFIRM_NETWORK_FEE_TAG = "sendConfirm.networkFee"

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun SendFlowConfirmScreen(
    app: AppManager,
    walletManager: WalletManager,
    details: ConfirmDetails,
    sendState: SendState,
    payjoinWaiting: PayjoinStatus.Negotiating?,
    onSendWithoutPayjoin: (PayjoinSessionId) -> Unit,
    onBack: () -> Unit,
    onSwipeToSend: () -> Unit,
    payjoinIntent: PayjoinIntent? = null,
    isPayjoinOn: Boolean = false,
    onPayjoinChange: (Boolean) -> Unit = {},
) {
    var sheetState by remember { mutableStateOf<SheetState?>(null) }
    val address = details.sendingTo().spacedOut()

    // fiat amount calculation
    var fiatAmount by remember { mutableStateOf("---") }
    LaunchedEffect(app.prices) {
        app.prices?.let { prices ->
            val amount = details.sendingAmount()
            fiatAmount = walletManager.convertAndDisplayFiat(amount, prices)
        } ?: run {
            app.dispatch(org.bitcoinppl.cove_core.AppAction.UpdateFiatPrices)
        }
    }

    // derive state from managers (like iOS @Environment)
    val metadata = walletManager.walletMetadata
    val isBalanceHidden = !(metadata?.sensitiveVisible ?: false)
    val balanceAmount = walletManager.amountFmt(walletManager.balance.spendable())
    val balanceDenomination = walletManager.unit
    val sendingAmount = walletManager.amountFmt(details.sendingAmount())
    val sendingAmountDenomination = walletManager.unit
    val dollarEquivalentText = fiatAmount
    val networkFee = walletManager.amountFmtUnit(details.feeTotal())
    val willReceive = walletManager.amountFmtUnit(details.sendingAmount())

    // the receiver may take up to this offer from the change, so the fee and total show their most
    val feeContribution = payjoinIntent?.feeContribution?.takeIf { isPayjoinOn }
    val payjoinFeeHelp =
        feeContribution?.let {
            stringResource(R.string.payjoin_confirm_fee_help, walletManager.amountFmtUnit(it.max()))
        }
    val willPay =
        feeContribution?.let {
            stringResource(R.string.payjoin_confirm_up_to, walletManager.amountFmtUnit(it.maxSpendingAmount()))
        } ?: walletManager.amountFmtUnit(details.spendingAmount())

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
                    onToggleVisibility = {
                        walletManager.dispatch(WalletManagerAction.ToggleSensitiveVisibility)
                    },
                )
                BoxWithConstraints(
                    modifier =
                        Modifier
                            .fillMaxHeight()
                            .background(MaterialTheme.colorScheme.surface)
                            .padding(horizontal = 16.dp),
                ) {
                    Column(
                        modifier =
                            Modifier
                                .fillMaxWidth()
                                .heightIn(min = maxHeight)
                                .verticalScroll(rememberScrollState()),
                    ) {
                        Column(Modifier.testTag("sendFlowConfirm.topSection")) {
                            AmountWidget(
                                amount = sendingAmount,
                                denomination = sendingAmountDenomination,
                                dollarText = dollarEquivalentText,
                                metadata = metadata,
                            )
                            HorizontalDivider(color = MaterialTheme.colorScheme.outlineVariant, thickness = 1.dp)
                            SummaryWidget(
                                address = address,
                                networkFee = networkFee,
                                payjoinFeeHelp = payjoinFeeHelp,
                                willReceive = willReceive,
                                willPay = willPay,
                                onAddressClick = { sheetState = SheetState.AdvancedDetails },
                                payjoinToggle =
                                    if (payjoinIntent != null) {
                                        {
                                            PayjoinToggleRow(
                                                isOn = isPayjoinOn,
                                                // the payment's mode is fixed once its swipe starts it
                                                enabled = sendState is SendState.Idle,
                                                onChange = onPayjoinChange,
                                            )
                                        }
                                    } else {
                                        null
                                    },
                            )
                        }

                        FlexibleSpacer()

                        Column(Modifier.testTag("sendFlowConfirm.bottomSection")) {
                            payjoinWaiting?.let { status ->
                                PayjoinWaitingCard(
                                    status = status,
                                    onSendWithoutPayjoin = onSendWithoutPayjoin,
                                    modifier = Modifier.padding(bottom = 16.dp),
                                )
                            }

                            SwipeToSendStub(
                                text = stringResource(R.string.action_swipe_to_send),
                                sendState = sendState,
                                onComplete = onSwipeToSend,
                                containerColor = MaterialTheme.coveColors.swipeTrackBg,
                                targetContainerColor = MaterialTheme.coveColors.midnightBtn,
                                knobColor = MaterialTheme.coveColors.midnightBtn,
                                textColor = MaterialTheme.colorScheme.onSurface,
                                targetTextColor = SWIPE_BUTTON_TEXT_COLOR_TARGET,
                                modifier =
                                    Modifier
                                        .fillMaxWidth()
                                        .navigationBarsPadding()
                                        .padding(bottom = 24.dp)
                                        .testTag("sendFlowConfirm.swipeToSend"),
                            )

                            Spacer(
                                Modifier
                                    .height(1.dp)
                                    .testTag("sendFlowConfirm.bottomPadding"),
                            )
                        }
                    }
                }
            }
        }
    }

    // AdvancedDetails bottom sheet
    when (sheetState) {
        SheetState.AdvancedDetails -> {
            ModalBottomSheet(
                onDismissRequest = { sheetState = null },
                sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true),
                containerColor = MaterialTheme.colorScheme.surfaceContainerHigh,
            ) {
                SendFlowAdvancedDetailsView(
                    app = app,
                    walletManager = walletManager,
                    details = details,
                    onDismiss = { sheetState = null },
                )
            }
        }
        null -> {}
    }
}

@Composable
private fun BalanceWidget(
    amount: String,
    denomination: String,
    isHidden: Boolean,
    onToggleVisibility: () -> Unit,
) {
    val screenHeight = LocalConfiguration.current.screenHeightDp.dp
    val balanceHeight = (screenHeight * 0.18f).coerceIn(100.dp, 160.dp)

    Box(
        modifier =
            Modifier
                .fillMaxWidth()
                .height(balanceHeight)
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
                    fontSize = 13.sp,
                )
                Spacer(Modifier.height(4.dp))
                Row(verticalAlignment = Alignment.Bottom) {
                    Text(
                        text = if (isHidden) "••••••" else amount,
                        color = Color.White,
                        fontSize = 20.sp,
                        fontWeight = FontWeight.Bold,
                    )
                    Spacer(Modifier.size(6.dp))
                    Text(
                        denomination,
                        color = Color.White,
                        fontSize = 15.sp,
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
    amount: String,
    denomination: String,
    dollarText: String,
    metadata: WalletMetadata?,
) {
    val density = LocalDensity.current
    var unitLabelWidth by remember { mutableStateOf(0.dp) }

    Column(modifier = Modifier.fillMaxWidth()) {
        Spacer(Modifier.height(16.dp))
        Text(
            stringResource(R.string.label_you_are_sending),
            color = MaterialTheme.colorScheme.onSurface,
            fontSize = 17.sp,
            fontWeight = FontWeight.SemiBold,
        )
        Spacer(Modifier.height(4.dp))
        Text(
            stringResource(R.string.label_amount_they_will_receive),
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            fontSize = 13.sp,
        )
        Spacer(Modifier.height(24.dp))

        // amount display - centered with dynamic offset based on unit label width
        Column(
            modifier = Modifier.fillMaxWidth(),
            horizontalAlignment = Alignment.CenterHorizontally,
        ) {
            Row(
                verticalAlignment = Alignment.Bottom,
                modifier = Modifier.offset(x = unitLabelWidth / 2),
            ) {
                AutoSizeText(
                    text = amount,
                    color = MaterialTheme.colorScheme.onSurface,
                    maxFontSize = 48.sp,
                    minimumScaleFactor = 0.5f,
                    fontWeight = FontWeight.Bold,
                )

                Row(
                    verticalAlignment = Alignment.Bottom,
                    modifier =
                        Modifier
                            .padding(start = 8.dp, bottom = 10.dp)
                            .onSizeChanged { size ->
                                unitLabelWidth = with(density) { size.width.toDp() }
                            },
                ) {
                    Text(denomination, color = MaterialTheme.colorScheme.onSurface, fontSize = 17.sp, maxLines = 1)
                    Spacer(Modifier.width(4.dp))
                    Icon(
                        imageVector = Icons.Filled.ArrowDropDown,
                        contentDescription = null,
                        tint = MaterialTheme.colorScheme.onSurface,
                        modifier = Modifier.size(20.dp),
                    )
                }
            }

            Spacer(Modifier.height(12.dp))
            Text(
                dollarText,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                fontSize = 20.sp,
            )
        }
        Spacer(Modifier.height(28.dp))
        SendFlowAccountSection(metadata, modifier = Modifier.fillMaxWidth())
        Spacer(Modifier.height(24.dp))
    }
}

@Composable
@Suppress("LongParameterList")
private fun SummaryWidget(
    address: String,
    networkFee: String,
    payjoinFeeHelp: String?,
    willReceive: String,
    willPay: String,
    onAddressClick: () -> Unit,
    payjoinToggle: (@Composable () -> Unit)?,
) {
    Column(
        modifier = Modifier.fillMaxWidth(),
    ) {
        Spacer(Modifier.height(28.dp))
        SummaryAddressRow(address = address, onClick = onAddressClick)
        Spacer(Modifier.height(20.dp))
        payjoinToggle?.let { toggle ->
            toggle()
            Spacer(Modifier.height(20.dp))
        }

        SummaryKeyValueRow(
            label = stringResource(R.string.label_network_fee),
            value = networkFee,
            labelColor = MaterialTheme.colorScheme.onSurfaceVariant,
            valueColor = MaterialTheme.colorScheme.onSurfaceVariant,
            valueTag = SEND_CONFIRM_NETWORK_FEE_TAG,
            valueNote = payjoinFeeHelp,
            valueNoteTag = SEND_CONFIRM_PAYJOIN_FEE_HELP_TAG,
        )
        Spacer(Modifier.height(20.dp))
        SummaryKeyValueRow(
            label = stringResource(R.string.label_they_will_receive),
            value = willReceive,
            labelColor = MaterialTheme.colorScheme.onSurface,
            valueColor = MaterialTheme.colorScheme.onSurface,
            labelFontWeight = FontWeight.SemiBold,
            valueFontWeight = FontWeight.SemiBold,
        )
        Spacer(Modifier.height(20.dp))
        SummaryKeyValueRow(
            label = stringResource(R.string.label_you_will_pay),
            value = willPay,
            labelColor = MaterialTheme.colorScheme.onSurface,
            valueColor = MaterialTheme.colorScheme.onSurface,
            labelFontWeight = FontWeight.SemiBold,
            valueFontWeight = FontWeight.SemiBold,
            valueTag = SEND_CONFIRM_TOTAL_SPENDING_TAG,
        )
        Spacer(Modifier.height(20.dp))
    }
}

@Composable
private fun SummaryAddressRow(
    address: String,
    onClick: () -> Unit,
) {
    KeyValueRow(
        modifier =
            Modifier
                .fillMaxWidth()
                .clickable(onClick = onClick),
        valueWeight = 2f,
        labelContent = {
            Text(
                text = stringResource(R.string.label_address),
                modifier = Modifier.fillMaxWidth(),
                style = TextStyle(fontSize = 13.sp),
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        },
        valueContent = {
            AddressText(
                address = address,
                modifier = Modifier.fillMaxWidth(),
                style = TextStyle(fontSize = 13.sp, fontWeight = FontWeight.SemiBold),
                color = MaterialTheme.colorScheme.onSurface,
                textAlign = TextAlign.Right,
                maxLines = 4,
            )
        },
    )
}

/** A summary row, with an optional smaller [valueNote] line under its value */
@Composable
@Suppress("FunctionNaming", "LongParameterList")
private fun SummaryKeyValueRow(
    label: String,
    value: String,
    labelColor: Color,
    valueColor: Color,
    modifier: Modifier = Modifier.fillMaxWidth(),
    labelFontWeight: FontWeight = FontWeight.Normal,
    valueFontWeight: FontWeight = FontWeight.Normal,
    valueTag: String? = null,
    valueNote: String? = null,
    valueNoteTag: String? = null,
) {
    // a note makes the value two lines, so the label lines up with the first one
    KeyValueRow(
        modifier = modifier,
        labelWeight = 1f,
        valueWeight = 2f,
        verticalAlignment = if (valueNote == null) Alignment.CenterVertically else Alignment.Top,
        labelContent = {
            Text(
                text = label,
                modifier = Modifier.fillMaxWidth(),
                style = TextStyle(fontSize = 13.sp, fontWeight = labelFontWeight),
                color = labelColor,
            )
        },
        valueContent = {
            Column(modifier = Modifier.fillMaxWidth()) {
                Text(
                    text = value,
                    modifier = Modifier.fillMaxWidth().optionalTestTag(valueTag),
                    style = TextStyle(fontSize = 13.sp, fontWeight = valueFontWeight),
                    color = valueColor,
                    textAlign = TextAlign.Right,
                    maxLines = 4,
                )

                valueNote?.let { note ->
                    Spacer(Modifier.height(2.dp))
                    Text(
                        text = note,
                        modifier = Modifier.fillMaxWidth().optionalTestTag(valueNoteTag),
                        style = TextStyle(fontSize = 12.sp),
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                        textAlign = TextAlign.Right,
                    )
                }
            }
        },
    )
}

private fun Modifier.optionalTestTag(tag: String?): Modifier = tag?.let { testTag(it) } ?: this

/** The per-payment Payjoin switch, shown only for a payment the receiver can join */
@Composable
private fun PayjoinToggleRow(
    isOn: Boolean,
    enabled: Boolean,
    onChange: (Boolean) -> Unit,
) {
    // the whole row toggles and carries the checked state, so ui tests read it from the tag
    Row(
        modifier =
            Modifier
                .fillMaxWidth()
                .toggleable(value = isOn, enabled = enabled, role = Role.Switch, onValueChange = onChange)
                .testTag(SEND_CONFIRM_PAYJOIN_TOGGLE_TAG),
        horizontalArrangement = Arrangement.spacedBy(16.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Column(
            modifier = Modifier.weight(1f),
            verticalArrangement = Arrangement.spacedBy(2.dp),
        ) {
            Text(
                text = stringResource(R.string.payjoin_confirm_toggle_title),
                color = MaterialTheme.colorScheme.onSurface,
                fontSize = 13.sp,
                fontWeight = FontWeight.SemiBold,
            )
            Text(
                text = stringResource(R.string.payjoin_confirm_toggle_subtitle),
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                fontSize = 12.sp,
            )
        }

        Switch(checked = isOn, onCheckedChange = null, enabled = enabled)
    }
}

@Composable
private fun SwipeToSendStub(
    text: String,
    sendState: SendState,
    onComplete: () -> Unit,
    containerColor: Color,
    targetContainerColor: Color,
    knobColor: Color,
    textColor: Color,
    targetTextColor: Color,
    modifier: Modifier = Modifier,
) {
    val density = LocalDensity.current
    val knobSizeDp = 62.dp
    val knobSizePx = with(density) { knobSizeDp.toPx() }
    var trackWidthPx by remember { mutableFloatStateOf(0f) }
    var isDragging by remember { mutableStateOf(false) }
    var completed by remember { mutableStateOf(false) }
    var rawOffset by remember { mutableFloatStateOf(0f) }
    val animOffset = remember { Animatable(0f) }
    val maxOffsetPx = (trackWidthPx - knobSizePx).coerceAtLeast(0f)
    val currentOffset = if (isDragging) rawOffset else animOffset.value
    val progress = if (maxOffsetPx <= 0f) 0f else (currentOffset / maxOffsetPx).coerceIn(0f, 1f)
    val dragState =
        rememberDraggableState { delta ->
            if (!completed && sendState is SendState.Idle) {
                rawOffset = (rawOffset + delta).coerceIn(0f, maxOffsetPx)
            }
        }
    val scope = rememberCoroutineScope()
    val infinite = rememberInfiniteTransition(label = "pulse")
    val pulseAlpha by infinite.animateFloat(
        initialValue = 0.6f,
        targetValue = 1f,
        animationSpec = infiniteRepeatable(animation = tween(900), repeatMode = RepeatMode.Reverse),
        label = "pulseAlpha",
    )

    // snap to full when state changes to non-idle
    LaunchedEffect(sendState) {
        if (sendState !is SendState.Idle) {
            animOffset.snapTo(maxOffsetPx)
            rawOffset = maxOffsetPx
        } else {
            // reset when back to idle
            animOffset.snapTo(0f)
            rawOffset = 0f
            completed = false
        }
    }
    Box(
        modifier =
            modifier
                .height(64.dp)
                .clip(RoundedCornerShape(32.dp))
                .background(
                    androidx.compose.ui.graphics.lerp(
                        containerColor,
                        targetContainerColor,
                        progress,
                    ),
                ).onGloballyPositioned { coords ->
                    trackWidthPx = coords.size.width.toFloat()
                    rawOffset = rawOffset.coerceIn(0f, (trackWidthPx - knobSizePx).coerceAtLeast(0f))
                    scope.launch {
                        animOffset.snapTo(
                            animOffset.value.coerceIn(
                                0f,
                                (trackWidthPx - knobSizePx).coerceAtLeast(0f),
                            ),
                        )
                    }
                }.draggable(
                    state = dragState,
                    orientation = Orientation.Horizontal,
                    enabled = sendState is SendState.Idle,
                    onDragStarted = {
                        isDragging = true
                    },
                    onDragStopped = {
                        isDragging = false
                        scope.launch {
                            animOffset.snapTo(rawOffset)
                            if (rawOffset >= maxOffsetPx * SWIPE_COMPLETE_THRESHOLD) {
                                animOffset.snapTo(maxOffsetPx)
                                if (!completed) {
                                    completed = true
                                    onComplete()
                                }
                            } else {
                                animOffset.animateTo(0f, tween(SWIPE_RETURN_DURATION_MS))
                                rawOffset = 0f
                            }
                        }
                    },
                ),
        contentAlignment = Alignment.Center,
    ) {
        val displayColor =
            androidx.compose.ui.graphics
                .lerp(textColor, targetTextColor, progress)

        // show swipe text only when idle
        if (sendState is SendState.Idle) {
            Text(
                text = text,
                color = displayColor,
                fontSize = 17.sp,
                fontWeight = FontWeight.SemiBold,
                textAlign = TextAlign.Center,
                modifier = Modifier.align(Alignment.Center),
            )
        }

        // state overlay - matches iOS SwipeToSendView
        when (sendState) {
            is SendState.Idle -> Unit // handled above
            is SendState.Sending -> {
                Row(
                    modifier = Modifier.align(Alignment.Center),
                    horizontalArrangement = Arrangement.spacedBy(12.dp),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    Text(
                        text = "sending",
                        color = Color.White,
                        fontSize = 17.sp,
                        fontWeight = FontWeight.SemiBold,
                    )
                    ThreeDotsAnimation()
                }
            }
            is SendState.Sent -> {
                Row(
                    modifier = Modifier.align(Alignment.Center),
                    horizontalArrangement = Arrangement.spacedBy(12.dp),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    Text(
                        text = "sent",
                        color = Color.White,
                        fontSize = 17.sp,
                        fontWeight = FontWeight.SemiBold,
                    )
                    Icon(
                        imageVector = Icons.Default.Check,
                        contentDescription = null,
                        tint = Color.Green,
                    )
                }
            }
            is SendState.Error -> {
                Row(
                    modifier = Modifier.align(Alignment.Center),
                    horizontalArrangement = Arrangement.spacedBy(10.dp),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    Text(
                        text = "error",
                        color = Color.White,
                        fontSize = 17.sp,
                        fontWeight = FontWeight.SemiBold,
                    )
                    Icon(
                        imageVector = Icons.Default.Close,
                        contentDescription = null,
                        tint = Color.Red,
                    )
                }
            }
        }

        Box(
            modifier =
                Modifier
                    .size(knobSizeDp)
                    .align(Alignment.CenterStart)
                    .graphicsLayer { translationX = currentOffset }
                    .clip(CircleShape)
                    .background(knobColor),
            contentAlignment = Alignment.Center,
        ) {
            // only show arrow icon when idle
            if (sendState is SendState.Idle) {
                Icon(
                    imageVector = Icons.AutoMirrored.Filled.ArrowForward,
                    contentDescription = null,
                    tint = Color.White,
                    modifier = Modifier.graphicsLayer(alpha = if (!isDragging && progress == 0f) pulseAlpha else 1f),
                )
            }
        }
    }
}

// three dots animation for sending state
@Composable
private fun ThreeDotsAnimation() {
    val density = LocalDensity.current
    val dotLiftPx = with(density) { 6.dp.toPx() }

    Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
        repeat(3) { index ->
            val infiniteTransition = rememberInfiniteTransition(label = "dot$index")
            val progress by infiniteTransition.animateFloat(
                initialValue = 0f,
                targetValue = 1f,
                animationSpec =
                    infiniteRepeatable(
                        animation = tween(500, delayMillis = index * 200),
                        repeatMode = RepeatMode.Reverse,
                    ),
                label = "dotBounce$index",
            )
            val scale = 0.5f + (progress * 0.5f)
            Box(
                modifier =
                    Modifier
                        .size(6.dp)
                        .graphicsLayer {
                            scaleX = scale
                            scaleY = scale
                            translationY = -dotLiftPx * progress
                        }
                        .background(Color.White, CircleShape),
            )
        }
    }
}
