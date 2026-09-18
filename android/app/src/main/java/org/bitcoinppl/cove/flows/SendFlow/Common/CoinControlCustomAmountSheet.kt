@file:Suppress("PackageNaming")

package org.bitcoinppl.cove.flows.SendFlow.Common

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
import kotlin.math.roundToLong
import org.bitcoinppl.cove.WalletManager
import org.bitcoinppl.cove.flows.CoinControlFlow.displayDate
import org.bitcoinppl.cove.flows.CoinControlFlow.displayName
import org.bitcoinppl.cove.flows.SendFlow.SendFlowManager
import org.bitcoinppl.cove.ui.theme.CoveColor
import org.bitcoinppl.cove_core.SendFlowManagerAction
import org.bitcoinppl.cove_core.ffiConservativeDustLimitSats
import org.bitcoinppl.cove_core.types.Amount
import org.bitcoinppl.cove_core.types.Utxo
import org.bitcoinppl.cove_core.types.UtxoType

internal enum class PinState {
    NONE,
    SOFT,
    HARD,
}

/** Pin state and slider amount in sats after one raw slider event */
internal data class SmartSnap(
    val pinState: PinState,
    val amount: Double,
)

/**
 * Slider pins in sats, the slider holds at max and at the soft max just below it because
 * amounts between the two would leave a dust change output
 */
internal data class SmartSnapBand(
    val softMaxSend: Double,
    val maxSend: Double,
    val step: Double,
) {
    /** Applies the pins to one raw slider event */
    fun snap(
        pinState: PinState,
        raw: Double,
        previousRaw: Double,
    ): SmartSnap {
        val goingUp = raw > previousRaw
        val goingDown = raw < previousRaw
        val belowBand = raw < softMaxSend - step

        return when (pinState) {
            PinState.HARD ->
                when {
                    // a tap on the track lands far below the band in one event,
                    // holding the pin there would commit max instead of the tapped amount
                    belowBand -> SmartSnap(PinState.NONE, raw)
                    goingDown -> SmartSnap(PinState.SOFT, softMaxSend)
                    // hold at pin
                    else -> SmartSnap(PinState.HARD, maxSend)
                }

            PinState.SOFT ->
                when {
                    // crossing upward -> snap to hard
                    goingUp -> SmartSnap(PinState.HARD, maxSend)
                    // pulled a full step below band -> release pin
                    belowBand -> SmartSnap(PinState.NONE, raw)
                    // hold at pin
                    else -> SmartSnap(PinState.SOFT, softMaxSend)
                }

            PinState.NONE ->
                when {
                    raw < softMaxSend -> SmartSnap(PinState.NONE, raw)
                    goingUp -> SmartSnap(PinState.HARD, maxSend)
                    else -> SmartSnap(PinState.SOFT, softMaxSend)
                }
        }
    }
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

    // pin state for slider
    var pinState by remember { mutableStateOf(PinState.HARD) }

    // slider values are always sats, the Float round trip of the Compose slider
    // would leave sub-satoshi noise on unit-relative BTC values
    var previousAmount by remember { mutableDoubleStateOf(0.0) }
    var customAmount by remember { mutableDoubleStateOf(0.0) }
    var enteringAmount by remember { mutableStateOf<String?>(null) }
    var isEditing by remember { mutableStateOf(false) }

    // min/max calculations
    val conservativeDustLimitSats = remember { ffiConservativeDustLimitSats() }
    val minSend = conservativeDustLimitSats.toDouble()
    val step = 10.0

    val maxSend =
        remember(sendFlowManager.amount) {
            val maxSendSats =
                sendFlowManager
                    .maxSendMinusFees()
                    ?.asSats()
                    ?.takeIf { it > conservativeDustLimitSats }
                    ?: (conservativeDustLimitSats + 1_000uL)

            maxSendSats.toDouble()
        }

    val softMaxSend =
        remember(sendFlowManager.amount) {
            val softMaxSendSats = sendFlowManager.maxSendMinusFeesAndSmallUtxo()?.asSats() ?: conservativeDustLimitSats
            softMaxSendSats.toDouble().coerceAtLeast(minSend)
        }

    // follow the manager amount, except mid-drag where it would fight the thumb
    LaunchedEffect(sendFlowManager.amount) {
        if (isEditing) return@LaunchedEffect

        customAmount = sendFlowManager.amount?.asSats()?.toDouble() ?: maxSend
        previousAmount = customAmount
    }

    fun amountFromSliderValue(value: Double): Amount = Amount.fromSat(value.roundToLong().coerceAtLeast(0L).toULong())

    fun coinControlAmountChanged(value: Double): SendFlowManagerAction =
        SendFlowManagerAction.NotifyCoinControlAmountChanged(amountFromSliderValue(value))

    fun displayAmount(): String = walletManager.amountFmt(amountFromSliderValue(customAmount))

    fun handleSliderChange(raw: Double) {
        isEditing = true
        enteringAmount = null

        val snap = SmartSnapBand(softMaxSend, maxSend, step).snap(pinState, raw, previousAmount)
        pinState = snap.pinState

        // update model only on real change
        if (customAmount != snap.amount) {
            customAmount = snap.amount
            sendFlowManager.debouncedDispatch(coinControlAmountChanged(snap.amount), debounceDelayMs = 200)
        }

        previousAmount = raw
    }

    fun finishSliderEditing() {
        // no delay replaces the pending debounced dispatch with the released amount
        sendFlowManager.debouncedDispatch(coinControlAmountChanged(customAmount), debounceDelayMs = 0)
        isEditing = false
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
                        fontSize = 17.sp,
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
                        fontSize = 15.sp,
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
                                    fontSize = 15.sp,
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
                            walletManager.unit.uppercase(),
                            fontSize = 15.sp,
                            fontWeight = FontWeight.SemiBold,
                            color = MaterialTheme.colorScheme.onSurface,
                        )
                    }
                }

                Spacer(Modifier.height(8.dp))

                Text(
                    "Use the slider to set the amount.",
                    fontSize = 11.sp,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )

                Spacer(Modifier.height(12.dp))

                Slider(
                    value = customAmount.toFloat(),
                    onValueChange = { handleSliderChange(it.toDouble()) },
                    valueRange = minSend.toFloat()..maxSend.toFloat(),
                    onValueChangeFinished = { finishSliderEditing() },
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
                    fontSize = 13.sp,
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
                fontSize = 11.sp,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
            )
        }
        Column(horizontalAlignment = Alignment.End) {
            Text(
                displayAmount,
                fontWeight = FontWeight.Normal,
                fontSize = 13.sp,
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
