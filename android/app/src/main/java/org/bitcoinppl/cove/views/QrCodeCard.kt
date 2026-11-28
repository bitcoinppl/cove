package org.bitcoinppl.cove.views

import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.unit.dp
import org.bitcoinppl.cove.QrCodeGenerator

private const val QR_BITMAP_SIZE = 720

/** The white border, in modules, that the QR code standard asks scanners to be given */
private const val QR_QUIET_ZONE_MODULES = 4

/**
 * A QR code on a white card whose border is the four-module quiet zone the QR code standard asks for
 *
 * For a code drawn over a dark background, such as a sheet in dark mode, where a camera could
 * mistake the background for part of the code. The border is drawn in the code's own modules, so it
 * stays four modules wide however dense the text makes the code. [modifier] sizes the card
 */
@Composable
fun QrCodeCard(
    text: String,
    contentDescription: String,
    modifier: Modifier = Modifier,
) {
    val bitmap =
        remember(text) {
            QrCodeGenerator.generate(text, size = QR_BITMAP_SIZE, quietZoneModules = QR_QUIET_ZONE_MODULES)
        }

    Image(
        bitmap = bitmap.asImageBitmap(),
        contentDescription = contentDescription,
        contentScale = ContentScale.Fit,
        modifier =
            modifier
                .clip(RoundedCornerShape(12.dp))
                .background(Color.White),
    )
}
