package org.bitcoinppl.cove.flows.TapSignerFlow

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.unit.dp

/**
 * displays 6 circles representing PIN entry state
 * filled circles indicate entered digits
 */
@Composable
fun PinCirclesView(
    pinLength: Int,
    modifier: Modifier = Modifier,
    totalCircles: Int = 6,
) {
    Row(
        modifier = modifier,
        horizontalArrangement = Arrangement.spacedBy(20.dp),
    ) {
        repeat(totalCircles) { index ->
            PinCircle(isFilled = index < pinLength)
        }
    }
}

@Composable
private fun PinCircle(
    isFilled: Boolean,
    modifier: Modifier = Modifier,
) {
    Surface(
        modifier = modifier.size(18.dp),
        shape = CircleShape,
        color = if (isFilled) MaterialTheme.colorScheme.primary else Color.Transparent,
        border =
            androidx.compose.foundation.BorderStroke(
                width = 1.3.dp,
                color = MaterialTheme.colorScheme.primary,
            ),
    ) {}
}
