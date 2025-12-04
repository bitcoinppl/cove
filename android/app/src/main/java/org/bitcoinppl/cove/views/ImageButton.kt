package org.bitcoinppl.cove.views

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.BasicText
import androidx.compose.foundation.text.TextAutoSize
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Call
import androidx.compose.material3.ButtonColors
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.Icon
import androidx.compose.material3.LocalContentColor
import androidx.compose.material3.Surface
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.painter.Painter
import androidx.compose.ui.graphics.vector.rememberVectorPainter
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
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
    Surface(
        modifier =
            modifier
                .clip(RoundedCornerShape(10.dp))
                .clickable(enabled = enabled, onClick = onClick),
        shape = RoundedCornerShape(10.dp),
        color = colors.containerColor,
        contentColor = colors.contentColor,
    ) {
        Row(
            verticalAlignment = Alignment.CenterVertically,
            modifier =
                Modifier
                    .fillMaxWidth()
                    .padding(vertical = 18.dp, horizontal = 12.dp),
        ) {
            if (leadingIcon != null) {
                Icon(
                    painter = leadingIcon,
                    contentDescription = null,
                    modifier = Modifier.size(24.dp),
                )
                Spacer(Modifier.width(8.dp))
            }
            BasicText(
                text = text,
                maxLines = 1,
                autoSize =
                    TextAutoSize.StepBased(
                        minFontSize = 7.sp,
                        maxFontSize = 14.sp,
                        stepSize = 0.5.sp,
                    ),
                style =
                    TextStyle(
                        fontWeight = FontWeight.Medium,
                        color = LocalContentColor.current,
                        textAlign = TextAlign.Center,
                    ),
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
