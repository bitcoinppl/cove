package org.bitcoinppl.cove.views

import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Call
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonColors
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.Icon
import androidx.compose.material3.LocalContentColor
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.painter.Painter
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.tooling.preview.Preview
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import org.bitcoinppl.cove.ui.theme.CoveColor

@Composable
fun ImageButton(
    modifier: Modifier = Modifier,
    text: String,
    leadingIcon: Painter? = null,
    onClick: () -> Unit,
    colors: ButtonColors = ButtonDefaults.buttonColors(),
    enabled: Boolean = true,
) {
    val maxFontSize = 14.sp
    val minimumScaleFactor = 0.2f
    val baseIconSize = 24.dp

    Button(
        onClick = onClick,
        shape = RoundedCornerShape(10.dp),
        colors = colors,
        modifier = modifier,
        enabled = enabled,
        contentPadding = PaddingValues(vertical = 18.dp, horizontal = 12.dp),
    ) {
        BoxWithConstraints {
            // track the scale factor from AutoSizeText
            var scaleFactor by remember { mutableStateOf(1f) }
            val iconSize = baseIconSize * scaleFactor

            Row(
                verticalAlignment = Alignment.CenterVertically,
                modifier = Modifier.fillMaxWidth(),
            ) {
                if (leadingIcon != null) {
                    Icon(
                        painter = leadingIcon,
                        contentDescription = null,
                        modifier = Modifier.size(iconSize),
                        tint = LocalContentColor.current,
                    )
                    Spacer(Modifier.width(8.dp * scaleFactor))
                }
                AutoSizeText(
                    text = text,
                    fontWeight = FontWeight.Medium,
                    maxFontSize = maxFontSize,
                    minimumScaleFactor = minimumScaleFactor,
                    modifier = Modifier.weight(1f),
                    onTextLayout = { result ->
                        val computedSize = result.layoutInput.style.fontSize
                        scaleFactor = (computedSize.value / maxFontSize.value).coerceIn(minimumScaleFactor, 1f)
                    },
                )
            }
        }
    }
}

@Preview
@Composable
private fun PreviewImageButton() {
    ImageButton(
        text = "Call Now",
        leadingIcon =
            androidx.compose.ui.graphics.vector
                .rememberVectorPainter(Icons.Default.Call),
        onClick = {},
        colors =
            ButtonDefaults.buttonColors(
                containerColor = CoveColor.btnPrimary,
                contentColor = CoveColor.midnightBlue,
            ),
    )
}
