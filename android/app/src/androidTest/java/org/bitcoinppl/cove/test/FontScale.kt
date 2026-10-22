package org.bitcoinppl.cove.test

import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.unit.Density

/** Largest system font scale offered by common devices, such as Samsung's maximum */
const val LARGEST_SYSTEM_FONT_SCALE = 2f

/** Renders [content] as if the system font size were set to [fontScale] */
@Composable
fun WithFontScale(
    fontScale: Float,
    content: @Composable () -> Unit,
) {
    val density = LocalDensity.current
    CompositionLocalProvider(
        LocalDensity provides Density(density.density, fontScale),
        content = content,
    )
}
