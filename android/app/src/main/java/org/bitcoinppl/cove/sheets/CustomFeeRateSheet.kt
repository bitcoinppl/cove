package org.bitcoinppl.cove.sheets

import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
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
import org.bitcoinppl.cove_core.SendFlowAlertState
import org.bitcoinppl.cove_core.SendFlowException
import org.bitcoinppl.cove_core.types.FeeRate
import org.bitcoinppl.cove_core.types.FeeRateOptionWithTotalFee
import org.bitcoinppl.cove_core.types.FeeRateOptionsWithTotalFee
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

/**
 * custom fee rate sheet - allows user to set custom sats/vbyte with slider
 * ported from iOS SendFlowCustomFeeRateView.swift
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun CustomFeeRateSheet(
    app: AppManager,
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
        // guard against empty send flow state
        if (sendFlowManager.amount == null) return
        if (sendFlowManager.address == null) return

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
                            totalSats = feeRateOption.totalFee().asSats().toLong()
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
                text = "Custom Fee Rate",
                style = MaterialTheme.typography.titleMedium,
                fontWeight = FontWeight.Bold,
                modifier =
                    Modifier
                        .fillMaxWidth()
                        .padding(vertical = 8.dp),
                textAlign = androidx.compose.ui.text.style.TextAlign.Center,
            )

            Spacer(modifier = Modifier.height(20.dp))

            // fee rate input
            OutlinedTextField(
                value = feeRateText,
                onValueChange = { feeRateText = it },
                label = { Text("Fee Rate (sats/vB)") },
                keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Decimal),
                singleLine = true,
                modifier = Modifier.fillMaxWidth(),
            )

            Spacer(modifier = Modifier.height(12.dp))

            // slider
            Slider(
                value = feeRateFloat.coerceIn(1f, maxFeeRate),
                onValueChange = { feeRateText = String.format(Locale.US, "%.2f", it) },
                valueRange = 1f..maxFeeRate,
                modifier = Modifier.fillMaxWidth(),
            )

            Spacer(modifier = Modifier.height(8.dp))

            // fee range display
            Row(
                modifier = Modifier.fillMaxWidth(),
                horizontalArrangement = Arrangement.SpaceBetween,
            ) {
                Text(
                    text = "1 sat/vB",
                    fontSize = 12.sp,
                    color = MaterialTheme.colorScheme.secondary,
                )
                Text(
                    text = "${String.format(Locale.US, "%.2f", maxFeeRate)} sat/vB",
                    fontSize = 12.sp,
                    color = MaterialTheme.colorScheme.secondary,
                )
            }

            Spacer(modifier = Modifier.height(20.dp))

            // total fee display
            totalSats?.let { sats ->
                Card(
                    modifier = Modifier.fillMaxWidth(),
                    shape = RoundedCornerShape(12.dp),
                    colors =
                        CardDefaults.cardColors(
                            containerColor = MaterialTheme.colorScheme.surfaceVariant,
                        ),
                ) {
                    Column(
                        modifier =
                            Modifier
                                .fillMaxWidth()
                                .padding(16.dp),
                    ) {
                        Text(
                            text = "Total Network Fee",
                            fontSize = 14.sp,
                            color = MaterialTheme.colorScheme.secondary,
                        )
                        Spacer(modifier = Modifier.height(4.dp))
                        Text(
                            text = "$sats sats",
                            fontWeight = FontWeight.Bold,
                            fontSize = 18.sp,
                        )
                    }
                }
            }

            Spacer(modifier = Modifier.height(20.dp))

            // done button
            Button(
                onClick = onDismiss,
                modifier = Modifier.fillMaxWidth(),
                shape = RoundedCornerShape(12.dp),
            ) {
                Text("Done")
            }
        }
    }
}
