package org.bitcoinppl.cove.utils

import androidx.compose.material3.MaterialTheme
import androidx.compose.runtime.Composable
import androidx.compose.ui.graphics.Color
import org.bitcoinppl.cove.ui.theme.CoveColor
import org.bitcoinppl.cove.ui.theme.isLight
import org.bitcoinppl.cove_core.types.FfiColor
import org.bitcoinppl.cove_core.types.FfiOpacity

/**
 * Convert FfiColor from Rust to Android Compose Color (theme-aware)
 * Matches iOS implementation in WalletColor+Ext.swift
 */
@Composable
fun FfiColor.toColor(): Color = toColor(isDark = !MaterialTheme.colorScheme.isLight)

/**
 * Convert FfiColor from Rust to Android Compose Color with explicit dark mode flag.
 * Use this overload when not in a Composable context.
 */
fun FfiColor.toColor(isDark: Boolean): Color =
    when (this) {
        is FfiColor.Red -> Color.Red.withOpacity(v1)
        is FfiColor.Blue -> Color.Blue.withOpacity(v1)
        is FfiColor.Green -> {
            val green = if (isDark) CoveColor.SystemGreenDark else CoveColor.SystemGreenLight
            green.withOpacity(v1)
        }
        is FfiColor.Yellow -> Color.Yellow.withOpacity(v1)
        is FfiColor.Orange -> Color(0xFFFF9500).withOpacity(v1)
        is FfiColor.Purple -> Color(0xFFAF52DE).withOpacity(v1)
        is FfiColor.Pink -> Color(0xFFFF2D55).withOpacity(v1)
        is FfiColor.White -> Color.White.withOpacity(v1)
        is FfiColor.Black -> Color.Black.withOpacity(v1)
        is FfiColor.Gray -> Color.Gray.withOpacity(v1)
        is FfiColor.CoolGray -> CoveColor.coolGray.withOpacity(v1)
        is FfiColor.Custom -> {
            Color(
                red = v1.r.toFloat() / 255f,
                green = v1.g.toFloat() / 255f,
                blue = v1.b.toFloat() / 255f,
            ).withOpacity(v2)
        }
    }

/**
 * Apply FfiOpacity (0-100) to a Color
 * FfiOpacity is a percentage value where 100 = fully opaque
 */
fun Color.withOpacity(opacity: FfiOpacity): Color {
    if (opacity == 100.toUByte()) {
        return this
    }
    return this.copy(alpha = opacity.toFloat() / 100f)
}
