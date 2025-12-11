package org.bitcoinppl.cove.views

import androidx.compose.foundation.Canvas
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.size
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.Path
import androidx.compose.ui.graphics.drawscope.DrawScope
import androidx.compose.ui.graphics.drawscope.scale
import androidx.compose.ui.tooling.preview.Preview
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp

/**
 * Bitcoin shield icon with optional text alignment adjustment.
 *
 * @param alignWithText When true (default), applies a -5.dp vertical offset to visually
 *   align with adjacent text. Set to false when used in contexts where this adjustment
 *   is not needed.
 */
@Composable
fun BitcoinShieldIcon(
    modifier: Modifier = Modifier,
    size: Dp = 13.dp,
    color: Color = Color.White,
    alignWithText: Boolean = true,
) {
    val adjustedModifier =
        if (alignWithText) {
            modifier.offset(y = (-5).dp)
        } else {
            modifier
        }
    Canvas(modifier = adjustedModifier.size(size)) {
        val scaleFactor = size.toPx() / 125f
        scale(scaleFactor, scaleFactor) {
            drawShield(color)
            drawBitcoin(color)
        }
    }
}

private fun DrawScope.drawShield(color: Color) {
    val path =
        Path().apply {
            // outer shield
            moveTo(51.625f, 124.688f)
            cubicTo(50.5f, 124.688f, 48.875f, 124.188f, 47.5f, 123.375f)
            cubicTo(12.625f, 102.938f, 0.937f, 95.5f, 0.937f, 74f)
            lineTo(0.937f, 26.5f)
            cubicTo(0.937f, 19.812f, 3.375f, 17.562f, 9.125f, 15.312f)
            cubicTo(18f, 11.813f, 37.813f, 4.25f, 46.688f, 1.376f)
            cubicTo(48.312f, 0.876f, 49.938f, 0.438f, 51.625f, 0.438f)
            cubicTo(53.313f, 0.438f, 54.938f, 0.814f, 56.625f, 1.376f)
            cubicTo(65.5f, 4.438f, 85.25f, 11.75f, 94.125f, 15.313f)
            cubicTo(99.875f, 17.625f, 102.312f, 19.813f, 102.312f, 26.5f)
            lineTo(102.312f, 74f)
            cubicTo(102.312f, 95.5f, 90.812f, 103.125f, 55.812f, 123.375f)
            cubicTo(54.375f, 124.188f, 52.812f, 124.688f, 51.625f, 124.688f)
            close()

            // inner shield cutout
            moveTo(51.625f, 116.125f)
            cubicTo(52.688f, 116.125f, 53.813f, 115.625f, 55.25f, 114.688f)
            cubicTo(84.188f, 96.562f, 94.688f, 91.5f, 94.688f, 72.5f)
            lineTo(94.688f, 27.937f)
            cubicTo(94.688f, 24.812f, 94.125f, 23.625f, 91.75f, 22.75f)
            cubicTo(83.312f, 19.687f, 62.937f, 12.125f, 54.625f, 8.875f)
            cubicTo(53.437f, 8.437f, 52.375f, 8.125f, 51.625f, 8.125f)
            cubicTo(50.875f, 8.125f, 49.812f, 8.375f, 48.625f, 8.875f)
            cubicTo(40.312f, 12.125f, 19.875f, 19.438f, 11.5f, 22.75f)
            cubicTo(9.187f, 23.688f, 8.562f, 24.813f, 8.562f, 27.938f)
            lineTo(8.562f, 72.5f)
            cubicTo(8.563f, 91.5f, 19f, 96.688f, 48f, 114.688f)
            cubicTo(49.438f, 115.625f, 50.625f, 116.125f, 51.625f, 116.125f)
            close()
        }
    drawPath(path, color)
}

private fun DrawScope.drawBitcoin(color: Color) {
    val path =
        Path().apply {
            // main bitcoin B shape
            moveTo(40.22f, 85.453f)
            cubicTo(37.818f, 85.453f, 36.47f, 83.813f, 36.47f, 81.703f)
            lineTo(36.47f, 37.728f)
            cubicTo(36.47f, 35.502f, 37.965f, 33.978f, 40.22f, 33.978f)
            lineTo(43.62f, 33.978f)
            lineTo(43.62f, 28.324f)
            cubicTo(43.62f, 27.211f, 44.41f, 26.42f, 45.553f, 26.42f)
            cubicTo(46.695f, 26.42f, 47.486f, 27.21f, 47.486f, 28.324f)
            lineTo(47.486f, 33.978f)
            lineTo(52.994f, 33.978f)
            lineTo(52.994f, 28.324f)
            cubicTo(52.994f, 27.211f, 53.814f, 26.42f, 54.957f, 26.42f)
            cubicTo(56.041f, 26.42f, 56.832f, 27.21f, 56.832f, 28.324f)
            lineTo(56.832f, 34.096f)
            cubicTo(64.244f, 34.886f, 69.693f, 39.369f, 69.693f, 46.723f)
            cubicTo(69.693f, 52.26f, 65.709f, 57.182f, 60.231f, 58.148f)
            lineTo(60.231f, 58.471f)
            cubicTo(67.496f, 59.321f, 72.359f, 64.271f, 72.359f, 71.186f)
            cubicTo(72.359f, 80.268f, 65.621f, 84.721f, 56.832f, 85.366f)
            lineTo(56.832f, 91.313f)
            cubicTo(56.832f, 92.426f, 56.041f, 93.246f, 54.957f, 93.246f)
            cubicTo(53.815f, 93.246f, 52.994f, 92.426f, 52.994f, 91.313f)
            lineTo(52.994f, 85.453f)
            lineTo(47.486f, 85.453f)
            lineTo(47.486f, 91.313f)
            cubicTo(47.486f, 92.426f, 46.696f, 93.246f, 45.553f, 93.246f)
            cubicTo(44.41f, 93.246f, 43.619f, 92.426f, 43.619f, 91.313f)
            lineTo(43.619f, 85.453f)
            lineTo(40.221f, 85.453f)
            close()

            // upper B bump
            moveTo(42.273f, 56.391f)
            lineTo(51.5f, 56.391f)
            cubicTo(58.121f, 56.391f, 63.834f, 53.988f, 63.834f, 47.367f)
            cubicTo(63.834f, 41.537f, 59.117f, 39.047f, 53.141f, 39.047f)
            lineTo(42.27f, 39.047f)
            lineTo(42.27f, 56.39f)
            close()

            // lower B bump
            moveTo(42.27f, 80.384f)
            lineTo(53.607f, 80.384f)
            cubicTo(60.667f, 80.384f, 66.439f, 77.864f, 66.439f, 70.892f)
            cubicTo(66.439f, 63.714f, 60.169f, 61.429f, 52.933f, 61.429f)
            lineTo(42.272f, 61.429f)
            lineTo(42.272f, 80.384f)
            close()
        }
    drawPath(path, color)
}

@Preview(showBackground = true, backgroundColor = 0xFF1A1A2E)
@Composable
private fun BitcoinShieldIconPreview() {
    BitcoinShieldIcon(size = 50.dp, color = Color.White)
}
