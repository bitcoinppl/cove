// file level so the sheet's signature keeps matching its detekt baseline entries
@file:OptIn(ExperimentalComposeUiApi::class)

package org.bitcoinppl.cove.sheets

import androidx.compose.foundation.layout.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.ExperimentalComposeUiApi
import androidx.compose.ui.Modifier
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.testTagsAsResourceId
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.ensureActive
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import org.bitcoinppl.cove.AppManager
import org.bitcoinppl.cove.TaggedItem
import org.bitcoinppl.cove.WalletManager
import org.bitcoinppl.cove.flows.SendFlow.SendFlowManager
import org.bitcoinppl.cove.flows.SendFlow.SendFlowPresenter
import org.bitcoinppl.cove.ui.theme.title3
import org.bitcoinppl.cove.views.feerate.CustomFeeRateEditor
import org.bitcoinppl.cove.views.feerate.CustomFeeRateMessage
import org.bitcoinppl.cove.views.feerate.CustomFeeRateSlider
import org.bitcoinppl.cove.views.lockaware.ModalBottomSheet
import org.bitcoinppl.cove_core.SendFlowAlertState
import org.bitcoinppl.cove_core.SendFlowException
import org.bitcoinppl.cove_core.WalletManagerException
import org.bitcoinppl.cove_core.types.Amount
import org.bitcoinppl.cove_core.types.FeeRate
import org.bitcoinppl.cove_core.types.FeeRateOptionWithTotalFee
import org.bitcoinppl.cove_core.types.FeeRateOptionsWithTotalFee
import org.bitcoinppl.cove_core.types.FeeSpeed
import java.util.Locale
import kotlin.math.max
import kotlin.math.round
import org.bitcoinppl.cove.Log

private object CustomFeeRateConstants {
    // small adjustment to fee rate for error handling
    const val FEE_RATE_EPSILON = 0.01f

    // debounce delay for fee calculation
    const val FEE_CALC_DEBOUNCE_MS = 50L

    // delay before showing alert after dismissing sheet
    const val ALERT_DELAY_MS = 850L

    // typed rates round to hundredths of a sat/vB
    const val FEE_RATE_SCALE = 100f
}

/** Parses typed custom fee rate text, rounded to hundredths, or null when it isn't a finite number */
internal fun parseCustomFeeRate(text: String): Float? =
    text
        .trim()
        .toFloatOrNull()
        ?.takeIf { it.isFinite() }
        ?.let { round(it * CustomFeeRateConstants.FEE_RATE_SCALE) / CustomFeeRateConstants.FEE_RATE_SCALE }

// rust owns the minimum, so the check goes through FeeRate instead of a local constant
private fun customFeeRateInputError(feeRate: Float?): String? =
    when {
        feeRate == null -> "Enter the fee rate as a number"
        FeeRate.fromSatPerVb(feeRate).isBelowMinimum() -> "The fee rate must be at least 1 sat/vB"
        else -> null
    }

internal fun isTooHighCustomFeeError(error: SendFlowException): Boolean =
    when (error) {
        is SendFlowException.InsufficientFunds -> true
        is SendFlowException.WalletManager -> error.v1 is WalletManagerException.InsufficientFunds
        else -> false
    }

private fun estimatedCustomFeeOption(
    feeRate: Float,
    feeSpeed: FeeSpeed,
    selectedOption: FeeRateOptionWithTotalFee,
): FeeRateOptionWithTotalFee? {
    val selectedFee = selectedOption.totalFee()?.asSats()?.toDouble()
    val selectedRate = selectedOption.satPerVb().toDouble()
    val estimatedFee =
        if (selectedFee != null && selectedRate > 0) {
            (feeRate.toDouble() / selectedRate) * selectedFee
        } else {
            null
        }

    return estimatedFee?.let {
        FeeRateOptionWithTotalFee(
            feeSpeed = feeSpeed,
            feeRate = FeeRate.fromSatPerVb(feeRate),
            totalFee = Amount.fromSat(it.toULong()),
        )
    }
}

private fun customFeeTooHighAlert(): TaggedItem<SendFlowAlertState> =
    TaggedItem(
        SendFlowAlertState.General(
            title = "Fee too high!",
            message = "The fee rate you entered is too high, we automatically selected a lower fee",
        ),
    )

private suspend fun handleTooHighCustomFee(
    feeRate: Float,
    presenter: SendFlowPresenter,
    onDismiss: () -> Unit,
) {
    withContext(Dispatchers.Main) {
        presenter.erroredFeeRate = feeRate

        if (presenter.lastWorkingFeeRate != null) {
            onDismiss()
            delay(CustomFeeRateConstants.ALERT_DELAY_MS)
            presenter.alertState = customFeeTooHighAlert()
        }
    }
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

    val parsedFeeRate = remember(feeRateText) { parseCustomFeeRate(feeRateText) }
    val inputError = remember(parsedFeeRate) { customFeeRateInputError(parsedFeeRate) }
    val feeRateFloat = parsedFeeRate ?: selectedOption.satPerVb()

    // calculate fee speed based on current fee rate
    val feeSpeed =
        remember(feeRateFloat) {
            updatedFeeOptions.calculateCustomFeeSpeed(feeRate = feeRateFloat)
        }

    // max fee rate (3x fast or errored rate)
    val maxFeeRate =
        remember(updatedFeeOptions, presenter.erroredFeeRate) {
            val fast3 = updatedFeeOptions.fast().satPerVb() * 3
            val computed =
                presenter.erroredFeeRate
                    ?.let { minOf(it + CustomFeeRateConstants.FEE_RATE_EPSILON, fast3) }
                    ?: fast3
            max(1f, computed)
        }

    // get total sats with debouncing
    fun getTotalSatsDeduped(feeRate: Float) {
        // if amount is not set, we can't calculate fee
        if (sendFlowManager.amount == null) return

        // if address is not validated yet, estimate fee based on selected option's fee
        if (sendFlowManager.address == null) {
            val estimatedFeeOption =
                estimatedCustomFeeOption(
                    feeRate = feeRate,
                    feeSpeed = feeSpeed,
                    selectedOption = selectedOption,
                ) ?: return

            totalSats = estimatedFeeOption.totalFee()?.asSats()?.toLong()
            updatedFeeOptions = updatedFeeOptions.addCustomFeeRate(estimatedFeeOption)
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
                    } catch (e: SendFlowException) {
                        if (!isTooHighCustomFeeError(e)) {
                            // toString is the Rust Display text; the generated message lists raw fields
                            Log.e(
                                "CustomFeeRateSheet",
                                "Unexpected error calculating fee: ${e.javaClass.simpleName} - $e",
                                e,
                            )
                            return@withContext
                        }

                        // the rate may have changed or become invalid while Rust checked it
                        ensureActive()
                        handleTooHighCustomFee(
                            feeRate = feeRate,
                            presenter = presenter,
                            onDismiss = onDismiss,
                        )
                    } catch (e: Exception) {
                        // unexpected error during fee calculation
                        Log.e(
                            "CustomFeeRateSheet",
                            "Unexpected error calculating fee: ${e.javaClass.simpleName} - ${e.message}",
                            e,
                        )
                        withContext(Dispatchers.Main) {
                            // keep previous total sats value, don't crash
                        }
                    }
                }
            }
    }

    // trigger calculation when fee rate changes
    LaunchedEffect(feeRateFloat, inputError) {
        if (inputError == null) {
            getTotalSatsDeduped(feeRateFloat)
        } else {
            // a pending request for the previous rate must not update totals or dismiss the sheet
            feeCalculationJob?.cancel()
            feeCalculationJob = null
        }
    }

    // on dismiss, apply the custom fee option unless the typed rate is invalid
    DisposableEffect(Unit) {
        onDispose {
            val customFeeRate = parseCustomFeeRate(feeRateText) ?: return@onDispose
            if (customFeeRateInputError(customFeeRate) != null) return@onDispose

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
                "≈ ${walletManager.convertAndDisplayFiat(Amount.fromSat(sats.toULong()), prices)}"
            } ?: ""
        }

    ModalBottomSheet(
        onDismissRequest = onDismiss,
        containerColor = MaterialTheme.colorScheme.surface,
    ) {
        // a sheet is its own window, so its tags need exposing to UI tests here too
        Column(
            modifier =
                Modifier
                    .fillMaxWidth()
                    .semantics { testTagsAsResourceId = true }
                    .padding(horizontal = 16.dp)
                    .padding(bottom = 32.dp),
        ) {
            // title
            Text(
                text = "Set Custom Network Fee",
                style = MaterialTheme.typography.title3,
                fontWeight = FontWeight.Bold,
                modifier =
                    Modifier
                        .fillMaxWidth()
                        .padding(vertical = 12.dp),
                textAlign = androidx.compose.ui.text.style.TextAlign.Center,
            )

            Spacer(modifier = Modifier.height(20.dp))

            CustomFeeRateEditor(
                unitLabel = "satoshi/byte",
                feeRateText = feeRateText,
                onFeeRateTextChange = { feeRateText = it },
                feeSpeed = feeSpeed,
                slider =
                    CustomFeeRateSlider(
                        value = feeRateFloat,
                        range = 1f..maxFeeRate,
                        onValueChange = { feeRateText = String.format(Locale.US, "%.2f", it) },
                    ),
                totalText = totalSats?.let { "$it sats" },
                totalDetail = fiatAmount,
                message = inputError?.let { CustomFeeRateMessage.Error(it) },
                onDone = onDismiss,
            )
        }
    }
}
