package org.bitcoinppl.cove.sheets

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import org.bitcoinppl.cove.AppManager
import org.bitcoinppl.cove.SendFlowManager
import org.bitcoinppl.cove.SendFlowPresenter
import org.bitcoinppl.cove.TaggedItem
import org.bitcoinppl.cove.WalletManager
import org.bitcoinppl.cove.ui.theme.coveColors
import org.bitcoinppl.cove.utils.toColor
import org.bitcoinppl.cove_core.SendFlowAlertState
import org.bitcoinppl.cove_core.SendFlowException
import org.bitcoinppl.cove_core.types.Amount
import org.bitcoinppl.cove_core.types.FeeRate
import org.bitcoinppl.cove_core.types.FeeRateOptionWithTotalFee
import org.bitcoinppl.cove_core.types.FeeRateOptionsWithTotalFee
import org.bitcoinppl.cove_core.types.FeeSpeed
import org.bitcoinppl.cove_core.types.feeSpeedToCircleColor
import java.util.Locale
import kotlin.math.max
import kotlin.math.round

private object CustomFeeRateConstants {
    // small adjustment to fee rate for error handling
    const val FEE_RATE_EPSILON = 0.01f

    // debounce delay for fee calculation
    const val FEE_CALC_DEBOUNCE_MS = 50L

    // delay before showing alert after dismissing sheet
    const val ALERT_DELAY_MS = 850L
}

/** custom fee rate sheet - allows user to set custom sats/vbyte with slider */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun CustomFeeRateSheet(
    app: AppManager,
    walletManager: WalletManager,
    sendFlowManager: SendFlowManager,
    presenter: SendFlowPresenter,
    feeOptions: FeeRateOptionsWithTotalFee,
    selectedOption: FeeRateOptionWithTotalFee,
    onUpdateFeeOptions: (FeeRateOptionsWithTotalFee, FeeRateOptionWithTotalFee) -> Unit,
    onDismiss: () -> Unit,
) {
    var feeRateText by remember { mutableStateOf(selectedOption.feeRate().satPerVb().toString()) }
    var totalSats by remember { mutableStateOf<Long?>(null) }
    var feeCalculationJob by remember { mutableStateOf<Job?>(null) }
    var updatedFeeOptions by remember { mutableStateOf(feeOptions) }

    val scope = rememberCoroutineScope()

    // get fee rate as float
    val feeRateFloat =
        remember(feeRateText) {
            feeRateText.toFloatOrNull()?.let {
                round(it * 100f) / 100f
            } ?: selectedOption.satPerVb()
        }

    // calculate fee speed based on current fee rate
    val feeSpeed =
        remember(feeRateFloat) {
            updatedFeeOptions.calculateCustomFeeSpeed(feeRate = feeRateFloat)
        }

    // max fee rate (3x fast or errored rate)
    val maxFeeRate =
        remember(updatedFeeOptions, presenter.erroredFeeRate) {
            val fast3 = updatedFeeOptions.fast().satPerVb() * 3
            val computed = presenter.erroredFeeRate?.let { minOf(it + CustomFeeRateConstants.FEE_RATE_EPSILON, fast3) } ?: fast3
            max(1f, computed)
        }

    // get total sats with debouncing
    fun getTotalSatsDeduped(feeRate: Float) {
        // if amount is not set, we can't calculate fee
        if (sendFlowManager.amount == null) return

        // if address is not validated yet, estimate fee based on selected option's fee
        if (sendFlowManager.address == null) {
            val selectedFee = selectedOption.totalFee()?.asSats()?.toDouble() ?: return
            val selectedRate = selectedOption.satPerVb().toDouble()
            if (selectedRate > 0) {
                val estimatedFee = (feeRate.toDouble() / selectedRate) * selectedFee
                totalSats = estimatedFee.toLong()

                // create estimated custom fee option so it shows up when Done is pressed
                val estimatedFeeOption =
                    FeeRateOptionWithTotalFee(
                        feeSpeed = feeSpeed,
                        feeRate = FeeRate.fromSatPerVb(feeRate),
                        totalFee = Amount.fromSat(estimatedFee.toULong()),
                    )
                updatedFeeOptions = updatedFeeOptions.addCustomFeeRate(estimatedFeeOption)
            }
            return
        }

        feeCalculationJob?.cancel()
        feeCalculationJob =
            scope.launch {
                delay(CustomFeeRateConstants.FEE_CALC_DEBOUNCE_MS)

                withContext(Dispatchers.IO) {
                    try {
                        val feeRateObj = FeeRate.fromSatPerVb(feeRate)
                        val feeRateOption =
                            sendFlowManager.getNewCustomFeeRateWithTotal(
                                feeRate = feeRateObj,
                                feeSpeed = feeSpeed,
                            )

                        withContext(Dispatchers.Main) {
                            feeRateOption.totalFee()?.let { fee ->
                                totalSats = fee.asSats().toLong()
                            }
                            updatedFeeOptions = updatedFeeOptions.addCustomFeeRate(feeRateOption)
                            presenter.lastWorkingFeeRate = feeRate
                        }
                    } catch (e: SendFlowException.WalletManager) {
                        // handle insufficient funds - set max fee rate
                        withContext(Dispatchers.Main) {
                            presenter.erroredFeeRate = feeRate

                            if (presenter.lastWorkingFeeRate != null) {
                                onDismiss()
                                delay(CustomFeeRateConstants.ALERT_DELAY_MS)
                                presenter.alertState =
                                    TaggedItem(
                                        SendFlowAlertState.General(
                                            title = "Fee too high!",
                                            message = "The fee rate you entered is too high, we automatically selected a lower fee",
                                        ),
                                    )
                            }
                        }
                    } catch (e: Exception) {
                        // unexpected error during fee calculation
                        android.util.Log.e("CustomFeeRateSheet", "Unexpected error calculating fee: ${e.javaClass.simpleName} - ${e.message}", e)
                        withContext(Dispatchers.Main) {
                            // keep previous total sats value, don't crash
                        }
                    }
                }
            }
    }

    // trigger calculation when fee rate changes
    LaunchedEffect(feeRateFloat) {
        getTotalSatsDeduped(feeRateFloat)
    }

    // on dismiss, apply the custom fee option
    DisposableEffect(Unit) {
        onDispose {
            val customFeeRate = feeRateText.toFloatOrNull() ?: return@onDispose

            // if there's a matching non-custom option, use that instead
            updatedFeeOptions.getFeeRateWith(customFeeRate)?.let { matchingOption ->
                if (!matchingOption.isCustom()) {
                    val finalOptions = updatedFeeOptions.removeCustomFee()
                    onUpdateFeeOptions(finalOptions, matchingOption)
                    return@onDispose
                }
            }

            // otherwise use custom option
            updatedFeeOptions.custom()?.let { customOption ->
                onUpdateFeeOptions(updatedFeeOptions, customOption)
            }
        }
    }

    // calculate fiat amount
    val fiatAmount =
        remember(totalSats, app.prices) {
            val sats = totalSats ?: return@remember ""
            app.prices?.let { prices ->
                "≈ ${walletManager.rust.convertAndDisplayFiat(Amount.fromSat(sats.toULong()), prices)}"
            } ?: ""
        }

    ModalBottomSheet(
        onDismissRequest = onDismiss,
        containerColor = MaterialTheme.colorScheme.surface,
    ) {
        Column(
            modifier =
                Modifier
                    .fillMaxWidth()
                    .padding(horizontal = 16.dp)
                    .padding(bottom = 32.dp),
        ) {
            // title
            Text(
                text = "Set Custom Network Fee",
                style = MaterialTheme.typography.titleMedium,
                fontWeight = FontWeight.Bold,
                modifier =
                    Modifier
                        .fillMaxWidth()
                        .padding(vertical = 12.dp),
                textAlign = androidx.compose.ui.text.style.TextAlign.Center,
            )

            Spacer(modifier = Modifier.height(20.dp))

            Column(modifier = Modifier.fillMaxWidth()) {
                // "satoshi/byte" label
                Text(
                    text = "satoshi/byte",
                    fontWeight = FontWeight.Medium,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    fontSize = 14.sp,
                    modifier = Modifier.offset(y = 4.dp),
                )

                Spacer(modifier = Modifier.height(8.dp))

                // fee rate input + duration capsule row
                Row(
                    modifier = Modifier.fillMaxWidth(),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    BasicTextField(
                        value = feeRateText,
                        onValueChange = { feeRateText = it },
                        textStyle =
                            TextStyle(
                                fontSize = 34.sp,
                                fontWeight = FontWeight.SemiBold,
                                color = MaterialTheme.colorScheme.onSurface,
                            ),
                        keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Decimal),
                        singleLine = true,
                        modifier = Modifier.weight(1f),
                    )

                    Spacer(modifier = Modifier.width(8.dp))

                    // duration capsule
                    DurationCapsule(
                        speed = feeSpeed,
                        fontColor = MaterialTheme.colorScheme.onSurface,
                    )
                }

                Spacer(modifier = Modifier.height(8.dp))

                // slider
                Slider(
                    value = feeRateFloat.coerceIn(1f, maxFeeRate),
                    onValueChange = { feeRateText = String.format(Locale.US, "%.2f", it) },
                    valueRange = 1f..maxFeeRate,
                    modifier = Modifier.fillMaxWidth(),
                )

                Spacer(modifier = Modifier.height(4.dp))

                // total fee + fiat display
                Row(
                    modifier = Modifier.fillMaxWidth(),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    if (totalSats == null) {
                        CircularProgressIndicator(
                            modifier = Modifier.size(16.dp),
                            strokeWidth = 2.dp,
                            color = MaterialTheme.colorScheme.onSurface,
                        )
                    } else {
                        Text(
                            text = "$totalSats sats",
                            fontSize = 12.sp,
                            fontWeight = FontWeight.SemiBold,
                            color = MaterialTheme.colorScheme.onSurface,
                        )
                        Spacer(modifier = Modifier.width(8.dp))
                        Text(
                            text = fiatAmount,
                            fontSize = 11.sp,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                    }
                    Spacer(modifier = Modifier.weight(1f))
                }
            }

            Spacer(modifier = Modifier.height(20.dp))

            HorizontalDivider()

            Spacer(modifier = Modifier.height(20.dp))

            // done button
            Button(
                onClick = onDismiss,
                modifier =
                    Modifier
                        .fillMaxWidth()
                        .padding(horizontal = 28.dp),
                colors =
                    ButtonDefaults.buttonColors(
                        containerColor = MaterialTheme.coveColors.midnightBtn,
                        contentColor = Color.White,
                    ),
                shape = RoundedCornerShape(10.dp),
            ) {
                Text(
                    text = "Done",
                    fontSize = 13.sp,
                    fontWeight = FontWeight.SemiBold,
                    modifier = Modifier.padding(vertical = 8.dp),
                )
            }

            Spacer(modifier = Modifier.height(14.dp))
        }
    }
}

@Composable
private fun DurationCapsule(
    speed: FeeSpeed,
    fontColor: Color,
) {
    val durationText =
        remember(speed) {
            when (speed) {
                is FeeSpeed.Fast -> "~10 min"
                is FeeSpeed.Medium -> "~30 min"
                is FeeSpeed.Slow -> "~1 hour"
                is FeeSpeed.Custom -> {
                    val mins = speed.durationMins.toInt()
                    when {
                        mins < 60 -> "~$mins min"
                        mins < 120 -> "~1 hour"
                        else -> "~${mins / 60} hours"
                    }
                }
            }
        }

    val circleColor = feeSpeedToCircleColor(speed).toColor()

    Surface(
        shape = RoundedCornerShape(8.dp),
        color = Color.Gray.copy(alpha = 0.2f),
    ) {
        Row(
            modifier = Modifier.padding(horizontal = 8.dp, vertical = 4.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(6.dp),
        ) {
            Box(
                modifier =
                    Modifier
                        .size(8.dp)
                        .clip(CircleShape)
                        .background(circleColor),
            )
            Text(
                text = durationText,
                style = MaterialTheme.typography.labelSmall,
                fontWeight = FontWeight.SemiBold,
                color = fontColor,
            )
        }
    }
}
