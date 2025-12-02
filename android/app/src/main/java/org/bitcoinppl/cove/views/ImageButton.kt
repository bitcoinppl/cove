package org.bitcoinppl.cove.views

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
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.painter.Painter
import androidx.compose.ui.graphics.vector.rememberVectorPainter
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
    Button(
        onClick = onClick,
        shape = RoundedCornerShape(10.dp),
        colors = colors,
        modifier = modifier,
        enabled = enabled,
        contentPadding = PaddingValues(vertical = 18.dp, horizontal = 12.dp),
    ) {
        Row(
            verticalAlignment = Alignment.CenterVertically,
            modifier = Modifier.fillMaxWidth(),
        ) {
            if (leadingIcon != null) {
                Icon(
                    painter = leadingIcon,
                    contentDescription = null,
                    modifier = Modifier.size(24.dp),
                    tint = LocalContentColor.current,
                )
                Spacer(Modifier.width(8.dp))
            }
            AutoSizeText(
                text = text,
                fontWeight = FontWeight.Medium,
                maxFontSize = 14.sp,
                minimumScaleFactor = 0.5f,
                modifier = Modifier.weight(1f),
            )
        }
    }
}

@Preview
@Composable
private fun PreviewImageButton() {
    ImageButton(
        text = "Call Now",
        leadingIcon = rememberVectorPainter(Icons.Default.Call),
        onClick = {},
        colors =
            ButtonDefaults.buttonColors(
                containerColor = CoveColor.btnPrimary,
                contentColor = CoveColor.midnightBlue,
            ),
    )
}
