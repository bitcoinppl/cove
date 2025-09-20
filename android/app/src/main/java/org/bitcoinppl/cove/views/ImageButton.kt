package org.bitcoinppl.cove.views

import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Call
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonColors
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.Icon
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.tooling.preview.Preview
import androidx.compose.ui.unit.dp
import org.bitcoinppl.cove.ui.theme.BtnPrimary
import org.bitcoinppl.cove.ui.theme.MidnightBlue

@Composable
fun ImageButton(
    modifier: Modifier = Modifier,
    text: String,
    leading: @Composable (() -> Unit)? = null,
    onClick: () -> Unit,
    colors: ButtonColors = ButtonDefaults.buttonColors(),
) {
    Button(
        onClick = onClick,
        shape = RoundedCornerShape(10.dp),
        colors = colors,
        modifier = modifier,
        contentPadding = PaddingValues(vertical = 18.dp, horizontal = 12.dp),
    ) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            if (leading != null) {
                leading()
                Spacer(Modifier.width(8.dp))
            }
            Text(
                text,
                fontWeight = FontWeight.Medium,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis
            )
        }
    }
}

@Preview
@Composable
private fun PreviewImageButton() {
    ImageButton(
        text = "Call Now",
        leading = {
            Icon(Icons.Default.Call, contentDescription = null)
        },
        onClick = {},
        colors = ButtonDefaults.buttonColors(
            containerColor = BtnPrimary, contentColor = MidnightBlue
        ),
    )
}