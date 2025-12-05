package org.bitcoinppl.cove.ui.theme

import android.os.Build
import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.material3.ColorScheme
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.darkColorScheme
import androidx.compose.material3.dynamicDarkColorScheme
import androidx.compose.material3.dynamicLightColorScheme
import androidx.compose.material3.lightColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.SideEffect
import androidx.compose.ui.graphics.luminance
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalView
import androidx.core.view.WindowCompat
import org.bitcoinppl.cove.findActivity

/**
 * Extension to check if the current ColorScheme is light mode.
 * Uses surface luminance to reliably detect theme (works with dynamic colors).
 */
val ColorScheme.isLight: Boolean
    get() = this.surface.luminance() > 0.5f

private val DarkColorScheme =
    darkColorScheme(
        // Brand colors (Cove identity)
        primary = CoveColor.midnightBlue,
        primaryContainer = CoveColor.duskBlue,
        secondary = CoveColor.pastelBlue,
        tertiary = CoveColor.pastelTeal,
        // Error colors
        error = CoveColor.ErrorRed,
        errorContainer = CoveColor.pastelRed,
        // Custom dark mode background (optional override)
        background = CoveColor.coveBgDark,
        // All other colors use Material Design defaults for native Android feel
    )

private val LightColorScheme =
    lightColorScheme(
        // Brand colors (Cove identity)
        primary = CoveColor.midnightBlue,
        primaryContainer = CoveColor.coveLightGray,
        secondary = CoveColor.pastelNavy,
        secondaryContainer = CoveColor.btnPrimary,
        tertiary = CoveColor.pastelTeal,
        tertiaryContainer = CoveColor.lightMint,
        // Error colors
        error = CoveColor.ErrorRed,
        errorContainer = CoveColor.pastelRed,
        // All other colors use Material Design defaults for native Android feel
    )

@Composable
fun CoveTheme(
    darkTheme: Boolean = isSystemInDarkTheme(),
    // Dynamic color is available on Android 12+
    dynamicColor: Boolean = true,
    content: @Composable () -> Unit,
) {
    val colorScheme =
        when {
            dynamicColor && Build.VERSION.SDK_INT >= Build.VERSION_CODES.S -> {
                val context = LocalContext.current
                if (darkTheme) dynamicDarkColorScheme(context) else dynamicLightColorScheme(context)
            }

            darkTheme -> DarkColorScheme
            else -> LightColorScheme
        }
    val view = LocalView.current
    if (!view.isInEditMode) {
        SideEffect {
            view.context.findActivity()?.let { activity ->
                val window = activity.window
                WindowCompat.getInsetsController(window, view).isAppearanceLightStatusBars = darkTheme
            }
        }
    }

    MaterialTheme(
        colorScheme = colorScheme,
        typography = Typography,
        content = content,
    )
}
