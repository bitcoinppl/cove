package org.bitcoinppl.cove.ui.theme

import android.app.Activity
import android.os.Build
import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.darkColorScheme
import androidx.compose.material3.dynamicDarkColorScheme
import androidx.compose.material3.dynamicLightColorScheme
import androidx.compose.material3.lightColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.SideEffect
import androidx.compose.ui.graphics.toArgb
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalView
import androidx.core.view.WindowCompat

private val DarkColorScheme =
    darkColorScheme(
        // Primary colors
        primary = CoveColor.midnightBlue,
        onPrimary = CoveColor.almostWhite,
        primaryContainer = CoveColor.duskBlue,
        onPrimaryContainer = CoveColor.almostWhite,
        // Secondary colors
        secondary = CoveColor.pastelBlue,
        onSecondary = CoveColor.midnightBlue,
        secondaryContainer = CoveColor.duskBlue,
        onSecondaryContainer = CoveColor.almostWhite,
        // Tertiary colors
        tertiary = CoveColor.pastelTeal,
        onTertiary = CoveColor.midnightBlue,
        tertiaryContainer = CoveColor.duskBlue,
        onTertiaryContainer = CoveColor.almostWhite,
        // Error colors
        error = CoveColor.ErrorRed,
        onError = CoveColor.BackgroundLight,
        errorContainer = CoveColor.pastelRed,
        onErrorContainer = CoveColor.BackgroundLight,
        // Background & Surface
        background = CoveColor.ListBackgroundDark,
        onBackground = CoveColor.TextPrimaryDark,
        surface = CoveColor.BackgroundDark,
        onSurface = CoveColor.TextPrimaryDark,
        // Surface variants
        surfaceVariant = CoveColor.SurfaceDark,
        onSurfaceVariant = CoveColor.TextSecondary,
        surfaceTint = CoveColor.pastelBlue,
        // Surface containers
        surfaceContainerLowest = CoveColor.BackgroundDark,
        surfaceContainerLow = CoveColor.ListCardDark,
        surfaceContainer = CoveColor.ListCardDark,
        surfaceContainerHigh = CoveColor.ListCardAlternative,
        surfaceContainerHighest = CoveColor.SurfaceDark,
        // Outlines
        outline = CoveColor.BorderDark,
        outlineVariant = CoveColor.DividerDarkAlpha,
        // Inverse colors
        inverseSurface = CoveColor.BackgroundLight,
        inverseOnSurface = CoveColor.TextPrimary,
        inversePrimary = CoveColor.pastelBlue,
        // Scrim
        scrim = CoveColor.midnightBlue,
    )

private val LightColorScheme =
    lightColorScheme(
        // Primary colors
        primary = CoveColor.midnightBlue,
        onPrimary = CoveColor.BackgroundLight,
        primaryContainer = CoveColor.coveLightGray,
        onPrimaryContainer = CoveColor.midnightBlue,
        // Secondary colors
        secondary = CoveColor.pastelNavy,
        onSecondary = CoveColor.BackgroundLight,
        secondaryContainer = CoveColor.btnPrimary,
        onSecondaryContainer = CoveColor.midnightBlue,
        // Tertiary colors
        tertiary = CoveColor.pastelTeal,
        onTertiary = CoveColor.BackgroundLight,
        tertiaryContainer = CoveColor.lightMint,
        onTertiaryContainer = CoveColor.midnightBlue,
        // Error colors
        error = CoveColor.ErrorRed,
        onError = CoveColor.BackgroundLight,
        errorContainer = CoveColor.pastelRed,
        onErrorContainer = CoveColor.midnightBlue,
        // Background & Surface
        background = CoveColor.ListBackgroundLight,
        onBackground = CoveColor.TextPrimaryLight,
        surface = CoveColor.BackgroundLight,
        onSurface = CoveColor.TextPrimaryLight,
        // Surface variants
        surfaceVariant = CoveColor.SurfaceLight,
        onSurfaceVariant = CoveColor.TextSecondary,
        surfaceTint = CoveColor.pastelNavy,
        // Surface containers
        surfaceContainerLowest = CoveColor.BackgroundLight,
        surfaceContainerLow = CoveColor.almostWhite,
        surfaceContainer = CoveColor.ListCardLight,
        surfaceContainerHigh = CoveColor.SurfaceLight,
        surfaceContainerHighest = CoveColor.coveLightGray,
        // Outlines
        outline = CoveColor.BorderLight,
        outlineVariant = CoveColor.DividerLight,
        // Inverse colors
        inverseSurface = CoveColor.BackgroundDark,
        inverseOnSurface = CoveColor.TextPrimaryDark,
        inversePrimary = CoveColor.pastelBlue,
        // Scrim
        scrim = CoveColor.midnightBlue,
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
            val window = (view.context as Activity).window
            window.statusBarColor = colorScheme.primary.toArgb()
            WindowCompat.getInsetsController(window, view).isAppearanceLightStatusBars = darkTheme
        }
    }

    MaterialTheme(
        colorScheme = colorScheme,
        typography = Typography,
        content = content,
    )
}
