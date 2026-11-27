@file:Suppress("PackageNaming")

package org.bitcoinppl.cove.flows.FeeBumpFlow

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.rememberModalBottomSheetState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.State
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.ExperimentalComposeUiApi
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.testTagsAsResourceId
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.delay
import org.bitcoinppl.cove.FeeBumpManager
import org.bitcoinppl.cove.customRateFailureMessage
import org.bitcoinppl.cove.Log
import org.bitcoinppl.cove.runCatchingCancellable
import org.bitcoinppl.cove.sheets.parseCustomFeeRate
import org.bitcoinppl.cove.views.feerate.CustomFeeRateEditor
import org.bitcoinppl.cove.views.feerate.CustomFeeRateMessage
import org.bitcoinppl.cove.views.feerate.CustomFeeRateSlider
import org.bitcoinppl.cove.views.lockaware.ModalBottomSheet
import org.bitcoinppl.cove_core.FeeBumpOptions
import org.bitcoinppl.cove_core.types.Amount
import org.bitcoinppl.cove_core.types.FeeRate
import org.bitcoinppl.cove_core.types.FeeRateOptionWithTotalFee
import java.util.Locale
import kotlin.math.max
import kotlin.math.roundToInt

private const val TAG = "SpeedUpCustomRateSheet"
private const val PRICE_DEBOUNCE_MS = 50L
private const val MAXIMUM_FAST_MULTIPLE = 3f
private const val SLIDER_STEP = 0.01f
private const val PERCENT = 100.0

/**
 * Sets a custom rate for a fee bump, which can't go below the lowest rate a replacement needs
 *
 * Done selects the rate, and so does swiping the sheet away with a usable rate, as the send flow's
 * custom sheet does. The close button leaves the selection as it was
 */
@OptIn(ExperimentalMaterial3Api::class, ExperimentalComposeUiApi::class)
@Composable
internal fun SpeedUpCustomRateSheet(
    manager: FeeBumpManager,
    options: FeeBumpOptions,
    fiat: (Amount) -> String?,
    onDismiss: () -> Unit,
) {
    val minimum = options.minimumFeeRate.satPerVb()
    val maximum = max(options.feeOptions.fast().satPerVb() * MAXIMUM_FAST_MULTIPLE, minimum + SLIDER_STEP)
    val initial = max(manager.selected?.feeRate()?.satPerVb() ?: minimum, minimum)

    var feeRateText by remember { mutableStateOf(String.format(Locale.US, "%.2f", initial)) }

    val typed = parseCustomFeeRate(feeRateText)
    val isBelowMinimum = typed == null || typed < minimum
    val feeSpeed =
        remember(typed) {
            options.feeOptions.calculateCustomFeeSpeed(typed ?: initial)
        }

    // the last rate Rust priced, which Done selects when it is the rate typed
    val price by rememberBumpRatePrice(manager, typed.takeUnless { isBelowMinimum })
    val priced = (price as? BumpRatePrice.Priced)?.option
    val failure = (price as? BumpRatePrice.Failed)?.takeIf { it.feeRate == typed }
    val total = priced?.totalFee()
    val message = failure?.let { CustomFeeRateMessage.Error(it.message) } ?: minimumMessage(options, isBelowMinimum)

    fun done() {
        val usable = typed?.takeUnless { isBelowMinimum || failure != null }
        usable?.let { selectBumpRate(manager, priced, it) }
        onDismiss()
    }

    ModalBottomSheet(
        onDismissRequest = ::done,
        sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true),
        containerColor = MaterialTheme.colorScheme.surface,
    ) {
        Column(
            modifier =
                Modifier
                    .fillMaxWidth()
                    .semantics { testTagsAsResourceId = true }
                    .testTag("speedUp.customSheet"),
        ) {
            Box(modifier = Modifier.fillMaxWidth().padding(end = 8.dp), contentAlignment = Alignment.CenterEnd) {
                FeeBumpCloseButton(onClick = onDismiss)
            }

            CustomFeeRateEditor(
                unitLabel = "sats/vbyte",
                feeRateText = feeRateText,
                onFeeRateTextChange = { feeRateText = it },
                feeSpeed = feeSpeed,
                slider =
                    CustomFeeRateSlider(
                        value = typed ?: initial,
                        range = minimum..maximum,
                        onValueChange = { feeRateText = String.format(Locale.US, "%.2f", it) },
                    ),
                totalText = total?.let { "${it.satsString()} sats" },
                totalDetail = total?.let { totalDetail(it, options.originalFee, fiat) }.orEmpty(),
                message = message,
                messageBelowTotal = true,
                showsTotal = failure == null,
                onDone = ::done,
                modifier = Modifier.padding(horizontal = 16.dp).padding(bottom = 16.dp),
            )
        }
    }
}

/** The fiat value and how much more than the current fee, as "≈ 1.34 USD · +400%" */
private fun totalDetail(
    total: Amount,
    original: Amount,
    fiat: (Amount) -> String?,
): String {
    val originalSats = original.asSats().toDouble()
    val percent =
        if (originalSats == 0.0) {
            null
        } else {
            val increase = (total.asSats().toDouble() - originalSats) / originalSats * PERCENT
            "+${increase.roundToInt()}%"
        }

    return listOfNotNull(fiat(total), percent).joinToString(" · ")
}

/** The last rate Rust priced: its option, or why the replacement can't pay it */
private sealed interface BumpRatePrice {
    data class Priced(
        val option: FeeRateOptionWithTotalFee,
    ) : BumpRatePrice

    data class Failed(
        val feeRate: Float,
        val message: String,
    ) : BumpRatePrice
}

/**
 * Prices [feeRate] once typing settles, holding the last result
 *
 * A rate that changes or turns invalid (null) while Rust prices the last one cancels that pricing.
 * A failure replaces the last priced option, whose total belongs to another rate
 */
@Composable
private fun rememberBumpRatePrice(
    manager: FeeBumpManager,
    feeRate: Float?,
): State<BumpRatePrice?> {
    val price = remember { mutableStateOf<BumpRatePrice?>(null) }

    LaunchedEffect(feeRate) {
        if (feeRate == null) return@LaunchedEffect

        delay(PRICE_DEBOUNCE_MS)
        runCatchingCancellable(TAG, "Unable to price a fee bump at $feeRate sat/vB") {
            manager.customFeeOption(FeeRate.fromSatPerVb(feeRate))
        }.onSuccess { price.value = BumpRatePrice.Priced(it) }
            .onFailure { error ->
                price.value = customRateFailureMessage(error)?.let { BumpRatePrice.Failed(feeRate, it) }
            }
    }

    return price
}

/** Selects [feeRate], pricing it first when Done came before the typed rate finished pricing */
private fun selectBumpRate(
    manager: FeeBumpManager,
    priced: FeeRateOptionWithTotalFee?,
    feeRate: Float,
) {
    if (priced != null && priced.feeRate().satPerVb() == feeRate) {
        manager.select(priced)
        return
    }

    manager.selectCustomFeeRate(FeeRate.fromSatPerVb(feeRate))
}

private fun minimumMessage(
    options: FeeBumpOptions,
    isBelowMinimum: Boolean,
): CustomFeeRateMessage {
    val text = "Must be at least ${options.minimumFeeRate.satsPerVbyteString()} to replace your current fee"

    return if (isBelowMinimum) CustomFeeRateMessage.Error(text) else CustomFeeRateMessage.Hint(text)
}
