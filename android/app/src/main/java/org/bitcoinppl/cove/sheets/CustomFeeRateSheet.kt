package org.bitcoinppl.cove.sheets

import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
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
import org.bitcoinppl.cove.SendFlowPresenter
import org.bitcoinppl.cove.WalletManager
import uniffi.cove_core_ffi.Amount
import uniffi.cove_core_ffi.FeeRate
import uniffi.cove_core_ffi.FeeSpeed
import uniffi.cove_core_ffi.SendFlowError

/**
 * custom fee rate sheet - allows user to set custom sats/vbyte with slider
 * ported from iOS SendFlowCustomFeeRateView.swift
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun CustomFeeRateSheet(
    app: AppManager,
    manager: WalletManager,
    presenter: SendFlowPresenter,
    feeOptions: FeeRateOptionsWithTotalFee,
    selectedOption: FeeRateOptionWithTotalFee,
    onUpdateFeeOptions: (FeeRateOptionsWithTotalFee, FeeRateOptionWithTotalFee) -> Unit,
    onDismiss: () -> Unit
) {
    var feeRateText by remember { mutableStateOf(selectedOption.feeRate().satPerVb().toString()) }
    var totalSats by remember { mutableStateOf<Long?>(null) }
    var feeCalculationJob by remember { mutableStateOf<Job?>(null) }
    var updatedFeeOptions by remember { mutableStateOf(feeOptions) }

    val scope = rememberCoroutineScope()

    // get fee rate as float
    val feeRateFloat = remember(feeRateText) {
        feeRateText.toFloatOrNull()?.let {
            (it * 100).toInt() / 100f // round to 2 decimals
        } ?: selectedOption.satPerVb()
    }

    // calculate fee speed based on current fee rate
    val feeSpeed = remember(feeRateFloat) {
        updatedFeeOptions.calculateCustomFeeSpeed(feeRate = feeRateFloat)
    }

    // max fee rate (3x fast or errored rate)
    val maxFeeRate = remember(updatedFeeOptions, presenter.erroredFeeRate) {
        val fast3 = updatedFeeOptions.fast().satPerVb() * 3
        presenter.erroredFeeRate?.let { minOf(it + 0.01f, fast3) } ?: fast3
    }

    // get total sats with debouncing
    fun getTotalSatsDeduped(feeRate: Float) {
        feeCalculationJob?.cancel()
        feeCalculationJob = scope.launch {
            delay(50) // debounce

            withContext(Dispatchers.IO) {
                try {
                    val feeRateObj = FeeRate.fromSatPerVb(feeRate)
                    val feeRateOption = manager.rust.sendFlowManager()?.getNewCustomFeeRateWithTotal(
                        feeRate = feeRateObj,
                        feeSpeed = feeSpeed
                    )

                    feeRateOption?.let { option ->
                        withContext(Dispatchers.Main) {
                            totalSats = option.totalFee().asSats().toLong()
                            updatedFeeOptions = updatedFeeOptions.addCustomFeeRate(option)
                            presenter.lastWorkingFeeRate = feeRate
                        }
                    }
                } catch (e: SendFlowError.WalletManagerError) {
                    // handle insufficient funds - set max fee rate
                    withContext(Dispatchers.Main) {
                        presenter.erroredFeeRate = feeRate

                        if (presenter.lastWorkingFeeRate != null) {
                            onDismiss()
                            delay(850)
                            presenter.alertState = TaggedItem(
                                SendFlowAlertState.General(
                                    title = "Fee too high!",
                                    message = "The fee rate you entered is too high, we automatically selected a lower fee"
                                )
                            )
                        }
                    }
                } catch (e: Exception) {
                    android.util.Log.e("CustomFeeRateSheet", "Unable to get total sats: ${e.message}")
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
        containerColor = MaterialTheme.colorScheme.surface
    ) {
        Column(
            modifier = Modifier
                .fillMaxWidth()
                .padding(horizontal = 16.dp)
                .padding(bottom = 32.dp)
        ) {
            // title
            Text(
                text = "Set Custom Network Fee",
                style = MaterialTheme.typography.titleMedium,
                fontWeight = FontWeight.Bold,
                modifier = Modifier
                    .fillMaxWidth()
                    .padding(vertical = 12.dp),
                textAlign = androidx.compose.ui.text.style.TextAlign.Center
            )

            Spacer(modifier = Modifier.height(20.dp))

            // fee rate input
            Column(
                modifier = Modifier.fillMaxWidth()
            ) {
                Row(
                    verticalAlignment = Alignment.CenterVertically,
                    modifier = Modifier.fillMaxWidth()
                ) {
                    Text(
                        text = "satoshi/byte",
                        style = MaterialTheme.typography.bodyMedium,
                        fontWeight = FontWeight.Medium,
                        color = MaterialTheme.colorScheme.onSurfaceVariant
                    )
                    Spacer(modifier = Modifier.weight(1f))
                }

                Spacer(modifier = Modifier.height(8.dp))

                Row(
                    verticalAlignment = Alignment.CenterVertically,
                    modifier = Modifier.fillMaxWidth()
                ) {
                    TextField(
                        value = feeRateText,
                        onValueChange = { feeRateText = it },
                        modifier = Modifier.weight(1f),
                        textStyle = LocalTextStyle.current.copy(
                            fontSize = 34.sp,
                            fontWeight = FontWeight.SemiBold
                        ),
                        keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Decimal),
                        colors = TextFieldDefaults.colors(
                            focusedContainerColor = Color.Transparent,
                            unfocusedContainerColor = Color.Transparent,
                            focusedIndicatorColor = Color.Transparent,
                            unfocusedIndicatorColor = Color.Transparent
                        ),
                        singleLine = true
                    )

                    Spacer(modifier = Modifier.width(8.dp))

                    // duration capsule
                    Surface(
                        shape = RoundedCornerShape(12.dp),
                        color = MaterialTheme.colorScheme.primaryContainer
                    ) {
                        Text(
                            text = when (feeSpeed) {
                                is FeeSpeed.Fast -> "~10 min"
                                is FeeSpeed.Medium -> "~30 min"
                                is FeeSpeed.Slow -> "~1 hour"
                                is FeeSpeed.Custom -> {
                                    val mins = feeSpeed.durationMins.toInt()
                                    when {
                                        mins < 60 -> "~$mins min"
                                        mins < 120 -> "~1 hour"
                                        else -> "~${mins / 60} hours"
                                    }
                                }
                            },
                            style = MaterialTheme.typography.bodySmall,
                            fontWeight = FontWeight.SemiBold,
                            modifier = Modifier.padding(horizontal = 12.dp, vertical = 6.dp)
                        )
                    }
                }

                Spacer(modifier = Modifier.height(12.dp))

                // slider
                Slider(
                    value = feeRateFloat.coerceIn(1f, maxFeeRate),
                    onValueChange = { newValue ->
                        feeRateText = String.format("%.2f", newValue)
                    },
                    valueRange = 1f..maxFeeRate,
                    steps = ((maxFeeRate - 1f) * 100).toInt() - 1, // 0.01 step size
                    modifier = Modifier.fillMaxWidth()
                )

                Spacer(modifier = Modifier.height(8.dp))

                // total fee display
                Row(
                    verticalAlignment = Alignment.CenterVertically
                ) {
                    if (totalSats == null) {
                        CircularProgressIndicator(
                            modifier = Modifier.size(16.dp),
                            strokeWidth = 2.dp
                        )
                    } else {
                        Text(
                            text = "$totalSats sats",
                            style = MaterialTheme.typography.labelMedium,
                            fontWeight = FontWeight.SemiBold
                        )

                        Spacer(modifier = Modifier.width(8.dp))

                        val fiatAmount = remember(totalSats, app.prices) {
                            app.prices?.let { prices ->
                                totalSats?.let { sats ->
                                    "≈ ${manager.rust.convertAndDisplayFiat(Amount.fromSat(sats.toULong()), prices)}"
                                } ?: ""
                            } ?: ""
                        }

                        Text(
                            text = fiatAmount,
                            style = MaterialTheme.typography.labelSmall,
                            color = MaterialTheme.colorScheme.onSurfaceVariant
                        )
                    }
                    Spacer(modifier = Modifier.weight(1f))
                }
            }

            Spacer(modifier = Modifier.height(20.dp))

            Divider()

            Spacer(modifier = Modifier.height(20.dp))

            // done button
            Button(
                onClick = onDismiss,
                modifier = Modifier
                    .fillMaxWidth()
                    .padding(horizontal = 24.dp),
                colors = ButtonDefaults.buttonColors(
                    containerColor = Color(0xFF1C1C1E)
                ),
                shape = RoundedCornerShape(10.dp)
            ) {
                Text(
                    text = "Done",
                    style = MaterialTheme.typography.bodySmall,
                    fontWeight = FontWeight.SemiBold,
                    modifier = Modifier.padding(vertical = 8.dp)
                )
            }
        }
    }
}
