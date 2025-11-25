package org.bitcoinppl.cove.send.send_confirmation

import android.util.Log
import androidx.compose.animation.core.Animatable
import androidx.compose.animation.core.RepeatMode
import androidx.compose.animation.core.animateFloat
import androidx.compose.animation.core.infiniteRepeatable
import androidx.compose.animation.core.rememberInfiniteTransition
import androidx.compose.animation.core.tween
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.gestures.Orientation
import androidx.compose.foundation.gestures.draggable
import androidx.compose.foundation.gestures.rememberDraggableState
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.automirrored.filled.ArrowForward
import androidx.compose.material.icons.filled.ArrowDropDown
import androidx.compose.material.icons.filled.Check
import androidx.compose.material.icons.filled.Close
import androidx.compose.material.icons.filled.CurrencyBitcoin
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
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.tooling.preview.Preview
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import kotlinx.coroutines.launch
import org.bitcoinppl.cove.R
import org.bitcoinppl.cove.SendState
import org.bitcoinppl.cove.ui.theme.CoveColor
import org.bitcoinppl.cove.views.AutoSizeText
import org.bitcoinppl.cove.views.BalanceAutoSizeText

// Animation duration in milliseconds for swipe button returning to start position when swipe is incomplete.
private const val SWIPE_RETURN_DURATION_MS = 500

// Progress threshold (0.0-1.0) at which swipe is considered complete and triggers the action.
private const val SWIPE_COMPLETE_THRESHOLD = 0.9f

// Target text color (white) that text animates to during swipe gesture.
private val SWIPE_BUTTON_TEXT_COLOR_TARGET = CoveColor.SwipeButtonText

// Target background color (cyan) that button background animates to during swipe gesture.
private val SWIPE_BUTTON_BG_COLOR_TARGET = CoveColor.SwipeButtonBg

@Preview(showBackground = true)
@Composable
private fun SendConfirmationScreenPreview() {
    SendConfirmationScreen(
        onBack = {},
        onSwipeToSend = { Log.d("SendConfirmationPreview", "Swipe completed in preview") },
        onToggleBalanceVisibility = {},
        isBalanceHidden = false,
        balanceAmount = "1,166,369",
        balanceDenomination = "sats",
        sendingAmount = "25,555",
        sendingAmountDenomination = "sats",
        dollarEquivalentText = "$28.88",
        accountShort = "560072A4",
        address = "tb1qt 5alnv 8pm66 hv2zd cdzxr kyqfn wpuh8 9zrey kx",
        networkFee = "1,128 sats",
        willReceive = "25,555 sats",
        willPay = "26,683 sats",
    )
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun SendConfirmationScreen(
    onBack: () -> Unit,
    onSwipeToSend: () -> Unit,
    sendState: SendState = SendState.Idle,
    onToggleBalanceVisibility: () -> Unit = {},
    isBalanceHidden: Boolean = false,
    balanceAmount: String,
    balanceDenomination: String,
    sendingAmount: String,
    sendingAmountDenomination: String,
    dollarEquivalentText: String,
    accountShort: String,
    address: String,
    networkFee: String,
    willReceive: String,
    willPay: String,
) {
    Scaffold(
        containerColor = MaterialTheme.colorScheme.surface,
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
                    onToggleVisibility = onToggleBalanceVisibility,
                )
                Column(
                    modifier =
                        Modifier
                            .fillMaxHeight()
                            .background(MaterialTheme.colorScheme.surface)
                            .padding(horizontal = 16.dp),
                ) {
                    AmountWidget(
                        amount = sendingAmount,
                        denomination = sendingAmountDenomination,
                        dollarText = dollarEquivalentText,
                        accountShort = accountShort,
                    )
                    HorizontalDivider(color = MaterialTheme.colorScheme.outlineVariant, thickness = 1.dp)
                    SummaryWidget(
                        address = address,
                        networkFee = networkFee,
                        willReceive = willReceive,
                        willPay = willPay,
                    )
                    Spacer(Modifier.weight(1f))
                    SwipeToSendStub(
                        text = stringResource(R.string.action_swipe_to_send),
                        sendState = sendState,
                        onComplete = onSwipeToSend,
                        containerColor = MaterialTheme.colorScheme.surfaceVariant,
                        targetContainerColor = SWIPE_BUTTON_BG_COLOR_TARGET,
                        knobColor = MaterialTheme.colorScheme.surface,
                        textColor = MaterialTheme.colorScheme.onSurface,
                        targetTextColor = SWIPE_BUTTON_TEXT_COLOR_TARGET,
                        modifier =
                            Modifier
                                .fillMaxWidth()
                                .padding(bottom = 24.dp),
                    )
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
                        modifier = Modifier.offset(y = (-4).dp),
                    )
                }
            }
            IconButton(
                onClick = onToggleVisibility,
                modifier = Modifier.offset(y = 8.dp, x = 8.dp),
            ) {
                Icon(
                    imageVector = if (isHidden) Icons.Filled.VisibilityOff else Icons.Filled.Visibility,
                    contentDescription = null,
                    tint = Color.White,
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
    accountShort: String,
) {
    Column(modifier = Modifier.fillMaxWidth()) {
        Spacer(Modifier.height(16.dp))
        Text(
            stringResource(R.string.label_you_are_sending),
            color = MaterialTheme.colorScheme.onSurface,
            fontSize = 16.sp,
            fontWeight = FontWeight.SemiBold,
        )
        Spacer(Modifier.height(4.dp))
        Text(
            stringResource(R.string.label_amount_they_will_receive),
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            fontSize = 14.sp,
        )
        Spacer(Modifier.height(24.dp))
        Row(
            modifier = Modifier.fillMaxWidth(),
            verticalAlignment = Alignment.Bottom,
        ) {
            Box(modifier = Modifier.weight(1f), contentAlignment = Alignment.Center) {
                BalanceAutoSizeText(
                    text = amount,
                    color = MaterialTheme.colorScheme.onSurface,
                    baseFontSize = 48.sp,
                    minimumScaleFactor = 0.5f,
                    fontWeight = FontWeight.Bold,
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
        Spacer(Modifier.height(12.dp))
        AutoSizeText(
            dollarText,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            maxFontSize = 20.sp,
            minimumScaleFactor = 0.90f,
            textAlign = TextAlign.Center,
            modifier = Modifier.fillMaxWidth(),
        )
        Spacer(Modifier.height(28.dp))
        Row(
            modifier = Modifier.fillMaxWidth(),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Icon(
                Icons.Filled.CurrencyBitcoin,
                contentDescription = null,
                tint = CoveColor.WarningOrange,
                modifier = Modifier.size(28.dp),
            )
            Spacer(Modifier.size(12.dp))
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
        Spacer(Modifier.height(24.dp))
    }
}

@Composable
private fun SummaryWidget(
    address: String,
    networkFee: String,
    willReceive: String,
    willPay: String,
) {
    Column(
        modifier = Modifier.fillMaxWidth(),
    ) {
        Spacer(Modifier.height(28.dp))
        KeyValueRow(
            key = stringResource(R.string.label_address),
            value = address,
            valueColor = MaterialTheme.colorScheme.onSurface,
            keyColor = MaterialTheme.colorScheme.onSurfaceVariant,
            boldValue = true,
        )
        Spacer(Modifier.height(20.dp))
        KeyValueRow(
            key = stringResource(R.string.label_network_fee),
            value = networkFee,
            valueColor = MaterialTheme.colorScheme.onSurfaceVariant,
            keyColor = MaterialTheme.colorScheme.onSurfaceVariant,
        )
        Spacer(Modifier.height(20.dp))
        KeyValueRow(
            key = stringResource(R.string.label_they_will_receive),
            value = willReceive,
            valueColor = MaterialTheme.colorScheme.onSurface,
            boldValue = true,
            boldKey = true,
            keyColor = MaterialTheme.colorScheme.onSurface,
        )
        Spacer(Modifier.height(20.dp))
        KeyValueRow(
            key = stringResource(R.string.label_you_will_pay),
            value = willPay,
            valueColor = MaterialTheme.colorScheme.onSurface,
            boldValue = true,
            boldKey = true,
            keyColor = MaterialTheme.colorScheme.onSurface,
        )
        Spacer(Modifier.height(20.dp))
    }
}

@Composable
private fun KeyValueRow(
    key: String,
    value: String,
    keyColor: Color,
    valueColor: Color,
    boldValue: Boolean = false,
    boldKey: Boolean = false,
) {
    Row(
        modifier = Modifier.fillMaxWidth(),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Text(
            key,
            color = keyColor,
            fontSize = 14.sp,
            fontWeight = if (boldKey) FontWeight.SemiBold else FontWeight.Normal,
            modifier = Modifier.weight(1f),
        )
        Text(
            value,
            color = valueColor,
            fontSize = 14.sp,
            fontWeight = if (boldValue) FontWeight.SemiBold else FontWeight.Normal,
            textAlign = TextAlign.Right,
            modifier = Modifier.weight(1f),
        )
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
                fontSize = 18.sp,
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
                        fontSize = 18.sp,
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
                        fontSize = 18.sp,
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
                        fontSize = 18.sp,
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
    Row(horizontalArrangement = Arrangement.spacedBy(4.dp)) {
        repeat(3) { index ->
            val infiniteTransition = rememberInfiniteTransition(label = "dot$index")
            val alpha by infiniteTransition.animateFloat(
                initialValue = 0.3f,
                targetValue = 1f,
                animationSpec =
                    infiniteRepeatable(
                        animation = tween(600, delayMillis = index * 200),
                        repeatMode = RepeatMode.Reverse,
                    ),
                label = "dotAlpha$index",
            )
            Box(
                modifier =
                    Modifier
                        .size(6.dp)
                        .graphicsLayer { this.alpha = alpha }
                        .background(Color.White, CircleShape),
            )
        }
    }
}
