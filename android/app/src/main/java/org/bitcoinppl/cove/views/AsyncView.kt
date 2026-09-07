package org.bitcoinppl.cove.views

import androidx.compose.foundation.layout.size
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.LocalTextStyle
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp

// text that shows a loading spinner when the value is null
@Composable
fun AsyncText(
    text: String?,
    modifier: Modifier = Modifier,
    color: Color = Color.Unspecified,
    style: TextStyle = LocalTextStyle.current,
    fontWeight: FontWeight? = null,
    spinnerSize: Dp = 16.dp,
    spinnerStrokeWidth: Dp = 2.dp,
) {
    if (text != null) {
        Text(
            text = text,
            modifier = modifier,
            color = color,
            style = style,
            fontWeight = fontWeight,
        )
    } else {
        CircularProgressIndicator(
            modifier = modifier.size(spinnerSize),
            strokeWidth = spinnerStrokeWidth,
            color = if (color != Color.Unspecified) color else Color.Gray,
        )
    }
}
