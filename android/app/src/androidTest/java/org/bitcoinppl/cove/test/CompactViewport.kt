package org.bitcoinppl.cove.test

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.test.DeviceConfigurationOverride
import androidx.compose.ui.test.ForcedSize
import androidx.compose.ui.unit.Density
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.DpSize
import androidx.compose.ui.unit.dp

/** A compact phone held upright or turned on its side, as the app doesn't lock its orientation */
enum class Viewport(
    val width: Dp,
    val height: Dp,
) {
    // a Galaxy S20 at its default display size is about 360dp wide, and 640dp tall is a short phone
    PORTRAIT(360.dp, 640.dp),
    LANDSCAPE(640.dp, 360.dp),
}

/**
 * Lays out [content] in [viewport] at [fontScale], in a box tagged [testTag]
 *
 * The test emulator is an upright Pixel 6, about 411dp wide, and a size modifier can't grow past
 * the window, so it would squeeze a landscape viewport to the window's width. `ForcedSize` lowers
 * the density until the viewport fits instead, so [content] lays out in the viewport's dp size, as
 * on a phone of that size. [onDensity] gets that density, which differs from the activity's, so
 * the test reads positions in the dp the layout used
 */
@Composable
fun CompactViewport(
    viewport: Viewport,
    fontScale: Float,
    testTag: String,
    onDensity: (Density) -> Unit = {},
    content: @Composable () -> Unit,
) {
    DeviceConfigurationOverride(DeviceConfigurationOverride.ForcedSize(DpSize(viewport.width, viewport.height))) {
        WithFontScale(fontScale) {
            onDensity(LocalDensity.current)

            Box(Modifier.fillMaxSize().testTag(testTag)) {
                content()
            }
        }
    }
}
