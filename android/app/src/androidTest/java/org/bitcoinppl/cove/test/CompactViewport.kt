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
import androidx.compose.ui.unit.DpSize
import androidx.compose.ui.unit.dp

/**
 * A compact phone held upright, the only way the app runs since it locks itself to portrait
 *
 * A Galaxy S20 at its default display size is about 360dp wide, and 640dp tall is a short phone
 */
val COMPACT_PHONE = DpSize(360.dp, 640.dp)

/**
 * Lays out [content] in a viewport of [size] at [fontScale], in a box tagged [testTag]
 *
 * A size modifier can't grow past the window, so on an emulator shorter than the viewport it
 * would squeeze the content. `ForcedSize` lowers the density until the viewport fits instead, so
 * [content] lays out in the viewport's dp size, as on a phone of that size. [onDensity] gets that
 * density, which can differ from the activity's, so the test reads positions in the dp the
 * layout used
 */
@Composable
fun CompactViewport(
    fontScale: Float,
    testTag: String,
    size: DpSize = COMPACT_PHONE,
    onDensity: (Density) -> Unit = {},
    content: @Composable () -> Unit,
) {
    DeviceConfigurationOverride(DeviceConfigurationOverride.ForcedSize(size)) {
        WithFontScale(fontScale) {
            onDensity(LocalDensity.current)

            Box(Modifier.fillMaxSize().testTag(testTag)) {
                content()
            }
        }
    }
}
