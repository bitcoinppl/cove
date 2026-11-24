package org.bitcoinppl.cove.views.feerate

import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Slider
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import org.bitcoinppl.cove.ui.theme.coveColors
import org.bitcoinppl.cove_core.types.FeeSpeed

private const val CAPSULE_BACKGROUND_ALPHA = 0.2f

/** A note under a custom fee rate about the lowest rate it may take */
sealed interface CustomFeeRateMessage {
    val text: String

    /** A standing reminder of the minimum, in gray */
    data class Hint(
        override val text: String,
    ) : CustomFeeRateMessage

    /** The typed rate can't be used, in red */
    data class Error(
        override val text: String,
    ) : CustomFeeRateMessage
}

/**
 * Edits a custom fee rate: a sats/vB field, a slider, how long the rate should take to confirm and
 * the total fee it pays, with Done below
 *
 * Holds no state, so the send flow and the Speed Up sheet each keep their own rules for a rate and
 * price it their own way while sharing the layout and test tags
 *
 * @param totalText the total fee, such as "1,410 sats", null shows a spinner while it prices
 * @param totalDetail what follows the total, such as "≈ 1.34 USD · +400%"
 * @param messageBelowTotal puts [message] under the total as a note, rather than right under the
 *   field, which suits a standing hint
 * @param showsTotal off while the rate failed to price, as [message] says why there is no total
 */
// each caller passes its own copy, rate rules and pricing, so the layout takes them all
@Composable
@Suppress("LongParameterList")
fun CustomFeeRateEditor(
    unitLabel: String,
    feeRateText: String,
    onFeeRateTextChange: (String) -> Unit,
    feeSpeed: FeeSpeed,
    slider: CustomFeeRateSlider,
    totalText: String?,
    totalDetail: String,
    message: CustomFeeRateMessage?,
    onDone: () -> Unit,
    modifier: Modifier = Modifier,
    messageBelowTotal: Boolean = false,
    showsTotal: Boolean = true,
) {
    Column(modifier = modifier.fillMaxWidth()) {
        CustomFeeRateField(
            unitLabel = unitLabel,
            feeRateText = feeRateText,
            onFeeRateTextChange = onFeeRateTextChange,
            feeSpeed = feeSpeed,
        )

        if (!messageBelowTotal) {
            message?.let { CustomFeeRateMessageText(it, Modifier.padding(top = 4.dp)) }
        }

        Spacer(modifier = Modifier.height(8.dp))

        Slider(
            value = slider.value.coerceIn(slider.range),
            onValueChange = slider.onValueChange,
            valueRange = slider.range,
            modifier =
                Modifier
                    .fillMaxWidth()
                    .testTag("customFeeRate.slider"),
        )

        Spacer(modifier = Modifier.height(4.dp))

        if (showsTotal) CustomFeeRateTotal(totalText = totalText, totalDetail = totalDetail)

        if (messageBelowTotal) {
            message?.let { CustomFeeRateMessageText(it, Modifier.padding(top = 6.dp)) }
        }

        Spacer(modifier = Modifier.height(20.dp))
        HorizontalDivider()
        Spacer(modifier = Modifier.height(20.dp))

        CustomFeeRateDoneButton(enabled = message !is CustomFeeRateMessage.Error, onClick = onDone)
        Spacer(modifier = Modifier.height(14.dp))
    }
}

@Composable
private fun CustomFeeRateField(
    unitLabel: String,
    feeRateText: String,
    onFeeRateTextChange: (String) -> Unit,
    feeSpeed: FeeSpeed,
) {
    Text(
        text = unitLabel,
        fontWeight = FontWeight.Medium,
        color = MaterialTheme.colorScheme.onSurfaceVariant,
        fontSize = 16.sp,
        modifier = Modifier.offset(y = 4.dp),
    )

    Spacer(modifier = Modifier.height(8.dp))

    Row(
        modifier = Modifier.fillMaxWidth(),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        BasicTextField(
            value = feeRateText,
            onValueChange = onFeeRateTextChange,
            textStyle =
                TextStyle(
                    fontSize = 34.sp,
                    fontWeight = FontWeight.SemiBold,
                    color = MaterialTheme.colorScheme.onSurface,
                ),
            keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Decimal),
            singleLine = true,
            modifier =
                Modifier
                    .weight(1f)
                    .testTag("customFeeRate.field"),
        )

        Spacer(modifier = Modifier.width(8.dp))

        FeeDurationCapsule(
            speed = feeSpeed,
            fontColor = MaterialTheme.colorScheme.onSurface,
            containerColor = Color.Gray.copy(alpha = CAPSULE_BACKGROUND_ALPHA),
            textStyle = MaterialTheme.typography.bodySmall,
            fontWeight = FontWeight.SemiBold,
        )
    }
}

@Composable
private fun CustomFeeRateDoneButton(
    enabled: Boolean,
    onClick: () -> Unit,
) {
    Button(
        onClick = onClick,
        enabled = enabled,
        modifier =
            Modifier
                .fillMaxWidth()
                .padding(horizontal = 28.dp)
                .testTag("customFeeRate.done"),
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
}

/** Where the custom fee rate slider sits and what it reaches */
data class CustomFeeRateSlider(
    val value: Float,
    val range: ClosedFloatingPointRange<Float>,
    val onValueChange: (Float) -> Unit,
)

@Composable
private fun CustomFeeRateTotal(
    totalText: String?,
    totalDetail: String,
) {
    Row(
        modifier = Modifier.fillMaxWidth(),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        if (totalText == null) {
            CircularProgressIndicator(
                modifier = Modifier.size(16.dp),
                strokeWidth = 2.dp,
                color = MaterialTheme.colorScheme.onSurface,
            )
        } else {
            Text(
                text = totalText,
                fontSize = 12.sp,
                fontWeight = FontWeight.SemiBold,
                color = MaterialTheme.colorScheme.onSurface,
                modifier = Modifier.testTag("customFeeRate.total"),
            )

            Spacer(modifier = Modifier.width(8.dp))

            Text(
                text = totalDetail,
                fontSize = 11.sp,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }

        Spacer(modifier = Modifier.weight(1f))
    }
}

@Composable
private fun CustomFeeRateMessageText(
    message: CustomFeeRateMessage,
    modifier: Modifier = Modifier,
) {
    val color =
        when (message) {
            is CustomFeeRateMessage.Hint -> MaterialTheme.colorScheme.onSurfaceVariant
            is CustomFeeRateMessage.Error -> MaterialTheme.colorScheme.error
        }

    Text(
        text = message.text,
        color = color,
        fontSize = 12.sp,
        modifier = modifier.testTag("customFeeRate.minimumMessage"),
    )
}
