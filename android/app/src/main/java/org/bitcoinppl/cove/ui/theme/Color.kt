package org.bitcoinppl.cove.ui.theme

import androidx.compose.material3.MaterialTheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.ReadOnlyComposable
import androidx.compose.runtime.staticCompositionLocalOf
import androidx.compose.ui.graphics.Color as ComposeColor

/**
 * Cove brand colors and wallet-specific colors
 *
 * For system colors (text, background, surface, dividers, etc.), use MaterialTheme.colorScheme.* instead.
 * This keeps Android feeling native while preserving Cove's brand identity through custom colors.
 */
object CoveColor {
    // Brand colors (Cove identity)
    val midnightBlue = ComposeColor(0xFF1C2536)
    val btnPrimary = ComposeColor(0xFFE5EAEF)
    val coveLightGray = ComposeColor(0xFFE5EAEF)
    val duskBlue = ComposeColor(0xFF3A4254)
    val bitcoinOrange = ComposeColor(0xFFFF9500)

    // system green colors (theme-aware)
    val SystemGreenLight = ComposeColor(0xFF34C759)
    val SystemGreenDark = ComposeColor(0xFF30D158)

    // Neutral colors
    val coolGray = ComposeColor(0xFFD4D8D4)
    val almostGray = ComposeColor(0xFF787880)
    val almostWhite = ComposeColor(0xFFEBEDF0)

    // Dark mode overrides (for custom dark theme)
    val coveBgDark = ComposeColor(0xFF191919)
    val midnightBtnDark = ComposeColor(0xFF4A4A4D)

    // Wallet colors - pastel palette (preferred)
    val beige = ComposeColor(0xFFFFB36E)
    val lightMint = ComposeColor(0xFFC5E5CD)
    val lightPastelYellow = ComposeColor(0xFFF0D16D)
    val pastelBlue = ComposeColor(0xFF369CFF)
    val pastelNavy = ComposeColor(0xFF3291AF)
    val pastelRed = ComposeColor(0xFFFF6868)
    val pastelTeal = ComposeColor(0xFF81D99A)
    val pastelYellow = ComposeColor(0xFFFFCD00)

    // Wallet colors - legacy (backward compatibility)
    val walletColorLightOrange = ComposeColor(0xFFF4AC6C)
    val walletColorLightBlue = ComposeColor(0xFF3596F5)
    val walletColorDarkBlue = ComposeColor(0xFF328CA8)
    val walletColorLightRed = ComposeColor(0xFFF46466)
    val walletColorYellow = ComposeColor(0xFFF4C502)
    val walletColorLightGreen = ComposeColor(0xFF7DD195)
    val walletColorBlue = ComposeColor(0xFF0276F5)
    val walletGreen = ComposeColor(0xFF34C058)
    val walletColorOrange = ComposeColor(0xFFF49003)
    val walletColorPurple = ComposeColor(0xFFA850D5)

    // Wallet-specific functional colors
    val TransactionReceived = ComposeColor(0xFF2DC24E)
    val FeeFast = ComposeColor(0xFF4CAF50)
    val FeeMedium = ComposeColor(0xFFFFEB3B)
    val FeeSlow = ComposeColor(0xFFFF9800)
    val FeeCustom = ComposeColor(0xFF3B82F6)

    // Special UI elements
    val SwipeButtonBg = ComposeColor(0xFF00BCD4)
    val SwipeButtonText = ComposeColor(0xFFFFFFFF)
    val LinkBlue = ComposeColor(0xFF007AFF)
    val WarningOrange = ComposeColor(0xFFF59E0B)
    val SuccessGreen = ComposeColor(0xFF4CAF50)
    val ErrorRed = ComposeColor(0xFFF44336)
}

/**
 * Theme-aware Cove colors that automatically adapt to light/dark mode.
 * Mirrors iOS asset catalog pattern where colors have light/dark variants.
 *
 * Access via MaterialTheme.coveColors (e.g., MaterialTheme.coveColors.midnightBtn)
 */
data class CoveColorScheme(
    val midnightBtn: ComposeColor,
    val systemGreen: ComposeColor,
)

val LightCoveColors =
    CoveColorScheme(
        midnightBtn = CoveColor.midnightBlue, // #1C2536
        systemGreen = CoveColor.SystemGreenLight, // #34C759
    )

val DarkCoveColors =
    CoveColorScheme(
        midnightBtn = CoveColor.midnightBtnDark, // #4A4A4D
        systemGreen = CoveColor.SystemGreenDark, // #30D158
    )

val LocalCoveColors = staticCompositionLocalOf { LightCoveColors }

val MaterialTheme.coveColors: CoveColorScheme
    @Composable
    @ReadOnlyComposable
    get() = LocalCoveColors.current
