package org.bitcoinppl.cove.views

import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.ColorFilter
import androidx.compose.ui.graphics.painter.Painter
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.tooling.preview.Preview
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import org.bitcoinppl.cove.R
import org.bitcoinppl.cove.ui.theme.CoveColor


@Composable
fun RoundRectImage(
    size: Dp? = 40.dp,
    backgroundColor: Color? = CoveColor.TextSecondary,
    painter: Painter,
    contentDescription: String? = null,
    cornerRadius: Dp? = 4.dp,
    imageTint: Color? = Color.White
) {
    Box(
        modifier = Modifier
            .size(size!!)
            .background(color = backgroundColor!!, shape = RoundedCornerShape(cornerRadius!!)),
        contentAlignment = Alignment.Center
    ) {
        Image(
            painter = painter,
            contentDescription = contentDescription,
            modifier = Modifier.fillMaxSize(fraction = 0.5f),
            contentScale = ContentScale.Fit,
            colorFilter = ColorFilter.tint(imageTint!!)
        )
    }
}

@Preview
@Composable
fun PreviewPainter() {
    RoundRectImage(
        painter = painterResource(id = R.drawable.icon_network),
        contentDescription = "Profile Icon",
    )
}

