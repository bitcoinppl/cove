package org.bitcoinppl.cove.components

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.tooling.preview.Preview
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import org.bitcoinppl.cove.ui.theme.CoveColor

@Composable
fun ConfirmationIndicatorView(
    current: Int,
    total: Int = 3,
    modifier: Modifier = Modifier,
) {
    Column(
        modifier = modifier,
        verticalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        Text(
            text = "Confirmations",
            color = MaterialTheme.colorScheme.secondary,
            fontSize = 14.sp,
        )

        Text(
            text = "$current of $total",
            fontWeight = FontWeight.Bold,
            fontSize = 16.sp,
        )

        Row(
            modifier = Modifier.fillMaxWidth(),
            horizontalArrangement = Arrangement.spacedBy(10.dp),
        ) {
            repeat(total) { index ->
                val backgroundColor =
                    if (index < current) {
                        CoveColor.SuccessGreen
                    } else {
                        MaterialTheme.colorScheme.secondary.copy(alpha = 0.3f)
                    }

                androidx.compose.foundation.layout.Box(
                    modifier =
                        Modifier
                            .weight(1f)
                            .height(8.dp)
                            .background(
                                color = backgroundColor,
                                shape = RoundedCornerShape(4.dp),
                            ),
                )
            }
        }
    }
}

@Preview(showBackground = true)
@Composable
private fun ConfirmationIndicatorPreview0() {
    ConfirmationIndicatorView(current = 0)
}

@Preview(showBackground = true)
@Composable
private fun ConfirmationIndicatorPreview1() {
    ConfirmationIndicatorView(current = 1)
}

@Preview(showBackground = true)
@Composable
private fun ConfirmationIndicatorPreview2() {
    ConfirmationIndicatorView(current = 2)
}

@Preview(showBackground = true)
@Composable
private fun ConfirmationIndicatorPreview3() {
    ConfirmationIndicatorView(current = 3)
}
