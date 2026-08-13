package org.bitcoinppl.cove.views

import android.graphics.Color
import androidx.compose.foundation.layout.width
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.asAndroidBitmap
import androidx.compose.ui.test.assertHeightIsEqualTo
import androidx.compose.ui.test.assertWidthIsEqualTo
import androidx.compose.ui.test.captureToImage
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.unit.dp
import androidx.test.ext.junit.runners.AndroidJUnit4
import org.bitcoinppl.cove.test.LayoutRegressionTest
import org.bitcoinppl.cove.ui.theme.CoveTheme
import org.junit.Assert.assertNotEquals
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

@LayoutRegressionTest
@RunWith(AndroidJUnit4::class)
class QrExportProgressIndicatorTest {
    @get:Rule
    val compose = createComposeRule()

    @Test
    fun manyFramesFitAndRenderAcrossAvailableWidth() {
        val availableWidth = 260.dp

        compose.setContent {
            CoveTheme(dynamicColor = false) {
                QrExportProgressIndicator(
                    qrCount = 250,
                    currentIndex = 249,
                    modifier = Modifier.width(availableWidth),
                )
            }
        }

        val indicator = compose.onNodeWithContentDescription("QR frame")

        indicator
            .assertWidthIsEqualTo(availableWidth)
            .assertHeightIsEqualTo(12.dp)

        val bitmap = indicator.captureToImage().asAndroidBitmap()
        val verticalCenter = bitmap.height / 2
        val firstFrameColor = bitmap.getPixel(0, verticalCenter)
        val lastFrameColor = bitmap.getPixel(bitmap.width - 1, verticalCenter)

        assertNotEquals(Color.TRANSPARENT, firstFrameColor)
        assertNotEquals(Color.TRANSPARENT, lastFrameColor)
        assertNotEquals(firstFrameColor, lastFrameColor)
    }
}
