package org.bitcoinppl.cove.views.feerate

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.selected
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import org.bitcoinppl.cove.WalletManager
import org.bitcoinppl.cove.ui.theme.coveColors
import org.bitcoinppl.cove.utils.toColor
import org.bitcoinppl.cove.views.AsyncText
import org.bitcoinppl.cove_core.PriceResponse
import org.bitcoinppl.cove_core.types.Amount
import org.bitcoinppl.cove_core.types.FeeRateOptionWithTotalFee
import org.bitcoinppl.cove_core.types.FeeSpeed
import org.bitcoinppl.cove_core.types.feeSpeedToCircleColor

private const val SELECTED_BACKGROUND_ALPHA = 0.8f
private const val SELECTED_BORDER_ALPHA = 0.75f
private const val UNSELECTED_BORDER_ALPHA = 0.3f
private const val CAPSULE_BACKGROUND_ALPHA = 0.2f
private const val MINUTES_PER_HOUR = 60
private const val TWO_HOURS_IN_MINUTES = 120

/**
 * One fee rate a transaction can pay: its speed, rate, total fee and that fee in fiat
 *
 * Fed by plain data, so the send flow's fee picker and the Speed Up sheet draw the same row. The
 * row's test tag is [testTagPrefix] followed by its speed, such as `speedUp.option.medium`
 *
 * @param fiatAmount the total fee in fiat, such as "≈ $1.07 USD", null while it loads
 * @param missingTotalText shown in place of the total when the option has none, null shows a
 *   spinner instead
 */
// the send flow and Speed Up each pass their own pricing, tags and missing-total copy
@Composable
@Suppress("LongParameterList")
fun FeeRateOptionRow(
    feeOption: FeeRateOptionWithTotalFee,
    isSelected: Boolean,
    fiatAmount: String?,
    testTagPrefix: String,
    onSelect: () -> Unit,
    missingTotalText: String? = null,
) {
    val shape = RoundedCornerShape(12.dp)
    val backgroundColor =
        if (isSelected) {
            MaterialTheme.coveColors.midnightBtn.copy(alpha = SELECTED_BACKGROUND_ALPHA)
        } else {
            MaterialTheme.colorScheme.surfaceVariant
        }

    val contentColor = if (isSelected) Color.White else MaterialTheme.colorScheme.onSurface
    val borderColor =
        if (isSelected) {
            MaterialTheme.colorScheme.primary.copy(alpha = SELECTED_BORDER_ALPHA)
        } else {
            MaterialTheme.colorScheme.onSurfaceVariant.copy(alpha = UNSELECTED_BORDER_ALPHA)
        }

    val totalFee = feeOption.totalFee()
    val totalText = totalFee?.let { "${it.satsString()} sats" } ?: missingTotalText

    // a row with no total shows the missing text alone, not a spinner where the fiat would be
    val fiatText = if (totalFee == null && missingTotalText != null) "" else fiatAmount

    Box(
        modifier =
            Modifier
                .fillMaxWidth()
                .clip(shape)
                .border(width = 1.dp, color = borderColor, shape = shape)
                .background(backgroundColor)
                .clickable(onClick = onSelect)
                .semantics { selected = isSelected }
                .testTag("$testTagPrefix.${feeOption.feeSpeed().testTagKey}")
                .padding(16.dp),
    ) {
        Row(
            modifier = Modifier.fillMaxWidth(),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            FeeRateOptionSpeed(feeOption, contentColor, Modifier.weight(1f))
            FeeRateOptionTotal(totalText, fiatText, contentColor)
        }
    }
}

@Composable
private fun FeeRateOptionSpeed(
    feeOption: FeeRateOptionWithTotalFee,
    contentColor: Color,
    modifier: Modifier,
) {
    Column(modifier = modifier) {
        Row(
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            Text(
                text = feeOption.feeSpeed().toString(),
                style = MaterialTheme.typography.titleMedium,
                fontWeight = FontWeight.Medium,
                color = contentColor,
            )

            FeeDurationCapsule(speed = feeOption.feeSpeed(), fontColor = contentColor)
        }

        Spacer(modifier = Modifier.height(4.dp))

        Text(
            text = feeOption.satsPerVbyteString(),
            style = MaterialTheme.typography.bodyMedium,
            color = contentColor,
        )
    }
}

@Composable
private fun FeeRateOptionTotal(
    totalText: String?,
    fiatText: String?,
    contentColor: Color,
) {
    Column(horizontalAlignment = Alignment.End) {
        AsyncText(
            text = totalText,
            style = MaterialTheme.typography.titleMedium,
            fontWeight = FontWeight.Medium,
            color = contentColor,
        )

        Spacer(modifier = Modifier.height(4.dp))

        AsyncText(
            text = fiatText,
            style = MaterialTheme.typography.bodyMedium,
            color = contentColor,
            spinnerSize = 12.dp,
            spinnerStrokeWidth = 1.5.dp,
        )
    }
}

/**
 * A capsule with a colored dot and how long [speed] should take to confirm, such as "~30 min"
 *
 * @param containerColor defaults to [fontColor] at a fifth of its opacity
 */
@Composable
fun FeeDurationCapsule(
    speed: FeeSpeed,
    fontColor: Color,
    containerColor: Color = fontColor.copy(alpha = CAPSULE_BACKGROUND_ALPHA),
    textStyle: TextStyle = MaterialTheme.typography.labelSmall,
    fontWeight: FontWeight? = null,
) {
    val circleColor = feeSpeedToCircleColor(speed).toColor()

    Surface(shape = RoundedCornerShape(8.dp), color = containerColor) {
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
                text = speed.durationText(),
                style = textStyle,
                fontWeight = fontWeight,
                color = fontColor,
            )
        }
    }
}

/** How long a rate at this speed should take to confirm, such as "~30 min" */
fun FeeSpeed.durationText(): String =
    when (this) {
        is FeeSpeed.Fast -> "~10 min"
        is FeeSpeed.Medium -> "~30 min"
        is FeeSpeed.Slow -> "~1 hour"
        is FeeSpeed.Custom -> {
            val mins = durationMins.toInt()
            when {
                mins < MINUTES_PER_HOUR -> "~$mins min"
                mins < TWO_HOURS_IN_MINUTES -> "~1 hour"
                else -> "~${mins / MINUTES_PER_HOUR} hours"
            }
        }
    }

/** Names the speed in test tags, which a custom rate's duration would make unpredictable */
val FeeSpeed.testTagKey: String
    get() =
        when (this) {
            is FeeSpeed.Fast -> "fast"
            is FeeSpeed.Medium -> "medium"
            is FeeSpeed.Slow -> "slow"
            is FeeSpeed.Custom -> "custom"
        }

/** [amount] in fiat as an approximation, such as "≈ $1.07 USD", null until prices load */
fun approximateFiat(
    prices: PriceResponse?,
    walletManager: WalletManager,
    amount: Amount,
): String? = prices?.let { "≈ ${walletManager.convertAndDisplayFiat(amount, it)}" }
