package org.bitcoinppl.cove.flows.SendFlow

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Close
import androidx.compose.material.icons.filled.Link
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import org.bitcoinppl.cove.WalletManager
import org.bitcoinppl.cove.flows.CoinControlFlow.displayDate
import org.bitcoinppl.cove.flows.CoinControlFlow.displayName
import org.bitcoinppl.cove.ui.theme.CoveColor
import org.bitcoinppl.cove_core.SendFlowManagerAction
import org.bitcoinppl.cove_core.types.Amount
import org.bitcoinppl.cove_core.types.Utxo
import org.bitcoinppl.cove_core.types.UtxoType

private enum class PinState {
    NONE,
    SOFT,
    HARD,
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun CoinControlCustomAmountSheet(
    sendFlowManager: SendFlowManager,
    walletManager: WalletManager,
    utxos: List<Utxo>,
    onDismiss: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = false)
    val scope = rememberCoroutineScope()
    val unit = walletManager.unit
    val isSats = unit == "sats"

    // pin state for slider
    var pinState by remember { mutableStateOf(PinState.HARD) }
    var previousAmount by remember { mutableDoubleStateOf(0.0) }
    var customAmount by remember { mutableDoubleStateOf(0.0) }
    var enteringAmount by remember { mutableStateOf<String?>(null) }
    var isEditing by remember { mutableStateOf(false) }

    // debounced dispatch
    var debounceJob: Job? by remember { mutableStateOf(null) }

    // min/max calculations
    val minSendSats = 546L
    val minSend = if (isSats) minSendSats.toDouble() else minSendSats / 100_000_000.0

    val step = if (isSats) 10.0 else 0.0000001

    val maxSend =
        remember(sendFlowManager.amount, unit) {
            val amount = sendFlowManager.rust.maxSendMinusFees() ?: Amount.fromSat(minSendSats.toULong() + 1000u)
            val value = if (isSats) amount.asSats().toDouble() else amount.asBtc()
            value.coerceAtLeast(minSend)
        }

    val softMaxSend =
        remember(sendFlowManager.amount, unit) {
            val amount = sendFlowManager.rust.maxSendMinusFeesAndSmallUtxo() ?: Amount.fromSat(minSendSats.toULong())
            val value = if (isSats) amount.asSats().toDouble() else amount.asBtc()
            value.coerceAtLeast(minSend)
        }

    // Initialize customAmount from manager
    LaunchedEffect(sendFlowManager.amount, unit) {
        sendFlowManager.amount?.let { amount ->
            customAmount = if (isSats) amount.asSats().toDouble() else amount.asBtc()
            previousAmount = customAmount
        } ?: run {
            customAmount = maxSend
            previousAmount = maxSend
        }
    }

    // Display amount helper
    fun displayAmount(amountStr: String? = null): String {
        val amountDouble =
            amountStr
                ?.let { sendFlowManager.rust.sanitizeBtcEnteringAmount(enteringAmount ?: "", it) }
                ?.replace(",", "")
                ?.toDoubleOrNull()

        val amount =
            when {
                amountDouble != null && isSats -> Amount.fromSat(amountDouble.toULong())
                amountDouble != null && !isSats ->
                    Amount.fromSat(
                        (amountDouble * 100_000_000).toULong(),
                    )
                isSats -> Amount.fromSat(customAmount.toULong())
                else -> Amount.fromSat((customAmount * 100_000_000).toULong())
            }

        return walletManager.amountFmt(amount)
    }

    // Smart snap binding for slider
    fun handleSliderChange(raw: Double) {
        enteringAmount = null
        val goingUp = raw > previousAmount
        val goingDown = raw < previousAmount
        var adjusted = raw

        when (pinState) {
            PinState.HARD -> {
                adjusted =
                    if (goingDown) {
                        pinState = PinState.SOFT
                        softMaxSend
                    } else {
                        // hold at pin
                        maxSend
                    }
            }
            PinState.SOFT -> {
                adjusted =
                    when {
                        // crossing upward -> snap to hard
                        goingUp -> {
                            pinState = PinState.HARD
                            maxSend
                        }
                        // pulled a full step below band -> release pin
                        raw < softMaxSend - step -> {
                            pinState = PinState.NONE
                            raw
                        }
                        // hold at pin
                        else -> softMaxSend
                    }
            }
            PinState.NONE -> {
                if (raw >= softMaxSend) {
                    pinState = if (goingUp) PinState.HARD else PinState.SOFT
                    adjusted = if (goingUp) maxSend else softMaxSend
                }
            }
        }

        // update model only on real change
        if (customAmount != adjusted) {
            customAmount = adjusted

            // debounced dispatch
            debounceJob?.cancel()
            debounceJob =
                scope.launch {
                    delay(200)
                    sendFlowManager.dispatch(SendFlowManagerAction.NotifyCoinControlAmountChanged(adjusted))
                }
        }

        previousAmount = raw
    }

    ModalBottomSheet(
        onDismissRequest = onDismiss,
        sheetState = sheetState,
        containerColor = MaterialTheme.colorScheme.surfaceContainerHigh,
        modifier = modifier,
    ) {
        Column(
            modifier =
                Modifier
                    .fillMaxWidth()
                    .padding(horizontal = 16.dp)
                    .padding(bottom = 24.dp),
        ) {
            // Header
            Row(
                modifier = Modifier.fillMaxWidth(),
                horizontalArrangement = Arrangement.SpaceBetween,
                verticalAlignment = Alignment.Top,
            ) {
                Column(modifier = Modifier.weight(1f).padding(top = 16.dp)) {
                    Text(
                        "Sending UTXO Details",
                        fontSize = 18.sp,
                        fontWeight = FontWeight.SemiBold,
                        color = MaterialTheme.colorScheme.onSurface,
                    )
                    Spacer(Modifier.height(4.dp))
                    Text(
                        "You are sending the following UTXOs to the recipient.",
                        fontSize = 12.sp,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }

                Box(
                    modifier =
                        Modifier
                            .size(32.dp)
                            .background(
                                MaterialTheme.colorScheme.onSurface.copy(alpha = 0.08f),
                                CircleShape,
                            ).clickable { onDismiss() },
                    contentAlignment = Alignment.Center,
                ) {
                    Icon(
                        imageVector = Icons.Default.Close,
                        contentDescription = "Close",
                        tint = MaterialTheme.colorScheme.onSurface.copy(alpha = 0.8f),
                        modifier = Modifier.size(16.dp),
                    )
                }
            }

            Spacer(Modifier.height(24.dp))
            HorizontalDivider(
                color = MaterialTheme.colorScheme.outlineVariant,
                thickness = 1.dp,
            )
            Spacer(Modifier.height(24.dp))

            // UTXO List
            Column(
                modifier =
                    Modifier
                        .fillMaxWidth()
                        .weight(1f, fill = false)
                        .verticalScroll(rememberScrollState()),
            ) {
                utxos.forEachIndexed { index, utxo ->
                    UtxoDetailRow(utxo = utxo, displayAmount = walletManager.amountFmt(utxo.amount))
                    if (index < utxos.lastIndex) {
                        Spacer(Modifier.height(8.dp))
                    }
                }
            }

            Spacer(Modifier.height(24.dp))

            // Amount setter section
            Column(
                modifier = Modifier.fillMaxWidth(),
            ) {
                Row(
                    modifier = Modifier.fillMaxWidth(),
                    horizontalArrangement = Arrangement.SpaceBetween,
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    Text(
                        "Set Amount",
                        fontSize = 14.sp,
                        fontWeight = FontWeight.SemiBold,
                        color = MaterialTheme.colorScheme.onSurface,
                    )

                    Row(verticalAlignment = Alignment.CenterVertically) {
                        BasicTextField(
                            value = enteringAmount ?: displayAmount(),
                            onValueChange = { newValue ->
                                enteringAmount = newValue
                                sendFlowManager.dispatch(
                                    SendFlowManagerAction.NotifyCoinControlEnteredAmountChanged(
                                        newValue,
                                        true,
                                    ),
                                )
                            },
                            textStyle =
                                TextStyle(
                                    fontSize = 14.sp,
                                    fontWeight = FontWeight.SemiBold,
                                    color = MaterialTheme.colorScheme.onSurface,
                                    textAlign = TextAlign.End,
                                ),
                            keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Decimal),
                            singleLine = true,
                            modifier = Modifier.widthIn(max = 100.dp),
                        )
                        Spacer(Modifier.width(4.dp))
                        Text(
                            if (isSats) "SATS" else "BTC",
                            fontSize = 14.sp,
                            fontWeight = FontWeight.SemiBold,
                            color = MaterialTheme.colorScheme.onSurface,
                        )
                    }
                }

                Spacer(Modifier.height(8.dp))

                Text(
                    "Use the slider to set the amount.",
                    fontSize = 12.sp,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )

                Spacer(Modifier.height(12.dp))

                Slider(
                    value = customAmount.toFloat(),
                    onValueChange = { handleSliderChange(it.toDouble()) },
                    valueRange = minSend.toFloat()..maxSend.toFloat(),
                    onValueChangeFinished = {
                        if (isEditing) {
                            scope.launch {
                                sendFlowManager.dispatch(
                                    SendFlowManagerAction.NotifyCoinControlAmountChanged(
                                        customAmount,
                                    ),
                                )
                            }
                        }
                    },
                    colors =
                        SliderDefaults.colors(
                            thumbColor = MaterialTheme.colorScheme.primary,
                            activeTrackColor = MaterialTheme.colorScheme.primary,
                            inactiveTrackColor = MaterialTheme.colorScheme.outlineVariant,
                        ),
                    modifier = Modifier.fillMaxWidth(),
                )
            }
        }
    }
}

@Composable
private fun UtxoDetailRow(
    utxo: Utxo,
    displayAmount: String,
) {
    Row(
        modifier =
            Modifier
                .fillMaxWidth()
                .background(MaterialTheme.colorScheme.surface, RoundedCornerShape(10.dp))
                .padding(16.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Column(modifier = Modifier.weight(1f)) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Text(
                    text = utxo.displayName,
                    fontWeight = FontWeight.Normal,
                    color = MaterialTheme.colorScheme.onSurface,
                    fontSize = 14.sp,
                )
                if (utxo.type == UtxoType.CHANGE) {
                    Spacer(Modifier.width(4.dp))
                    Icon(
                        imageVector = Icons.Filled.Link,
                        contentDescription = null,
                        tint = CoveColor.WarningOrange,
                        modifier = Modifier.size(16.dp),
                    )
                }
            }
            Spacer(Modifier.height(4.dp))
            Text(
                text = utxo.address.unformatted(),
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                fontSize = 12.sp,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
            )
        }
        Column(horizontalAlignment = Alignment.End) {
            Text(
                displayAmount,
                fontWeight = FontWeight.Normal,
                fontSize = 14.sp,
                color = MaterialTheme.colorScheme.onSurface,
            )
            Spacer(Modifier.height(4.dp))
            Text(
                utxo.displayDate,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                fontSize = 12.sp,
            )
        }
    }
}
