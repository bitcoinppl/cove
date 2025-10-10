package org.bitcoinppl.cove.ui.theme

import androidx.compose.runtime.Stable
import androidx.compose.ui.graphics.Color as ComposeColor

/**
 * Cove design tokens (matching iOS color names)
 * Access colors using CoveColor.colorName pattern (similar to Swift's Color.colorName)
 */
object CoveColor {
    // primary colors
    val midnightBlue = ComposeColor(0xFF1C2536)
    val btnPrimary = ComposeColor(0xFFE5EAEF)
    val coveLightGray = ComposeColor(0xFFE5EAEF)
    val duskBlue = ComposeColor(0xFF3A4254)

    // neutral colors
    val almostGray = ComposeColor(0xFF787880)
    val almostWhite = ComposeColor(0xFFEBEDF0)
    val coveBg = ComposeColor(0xFFFFFFFF)  // light mode
    val coveBgDark = ComposeColor(0xFF191919)  // dark mode
    val midnightBtn = ComposeColor(0xFF1C2536)  // light mode
    val midnightBtnDark = ComposeColor(0xFF4A4A4D)  // dark mode

    // wallet colors (pastel palette matching iOS)
    val beige = ComposeColor(0xFFFFB36E)
    val lightMint = ComposeColor(0xFFC5E5CD)
    val lightPastelYellow = ComposeColor(0xFFF0D16D)
    val pastelBlue = ComposeColor(0xFF369CFF)
    val pastelNavy = ComposeColor(0xFF3291AF)
    val pastelRed = ComposeColor(0xFFFF6868)
    val pastelTeal = ComposeColor(0xFF81D99A)
    val pastelYellow = ComposeColor(0xFFFFCD00)

    // legacy wallet colors (keep for backward compatibility, but prefer pastel variants above)
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

    // common UI colors
    val TextPrimary = ComposeColor(0xFF101010)
    val TextSecondary = ComposeColor(0xFF8F8F95)
    val TextGray = ComposeColor(0xFF818086)
    val DividerLight = ComposeColor(0xFFE5E5EA)
    val BackgroundLight = ComposeColor(0xFFFFFFFF)
    val BackgroundDark = ComposeColor(0xFF0D1B2A)
    val SurfaceLight = ComposeColor(0xFFF1F1F3)
    val SurfaceDark = ComposeColor(0xFF525C6B)
    val BorderLight = ComposeColor(0xFFD1D5DB)
    val BorderMedium = ComposeColor(0xFF6B7280)
    val BorderDark = ComposeColor(0xFF374151)
    val LinkBlue = ComposeColor(0xFF007AFF)
    val WarningOrange = ComposeColor(0xFFF59E0B)
    val SuccessGreen = ComposeColor(0xFF4CAF50)
    val ErrorRed = ComposeColor(0xFFF44336)
    val FeeFast = ComposeColor(0xFF4CAF50)
    val FeeMedium = ComposeColor(0xFFFFEB3B)
    val FeeSlow = ComposeColor(0xFFFF9800)
    val FeeCustom = ComposeColor(0xFF3B82F6)
    val SliderActive = ComposeColor(0xFF3B82F6)
    val SliderInactive = ComposeColor(0xFFD1D5DB)
    val SwipeButtonBg = ComposeColor(0xFF00BCD4)
    val SwipeButtonText = ComposeColor(0xFFFFFFFF)
    val ListBackgroundLight = ComposeColor(0xFFF2F2F7)
    val ListBackgroundDark = ComposeColor(0xFF151515)
    val ListCardLight = ComposeColor(0xFFFFFFFF)
    val ListCardDark = ComposeColor(0xFF3C3C3E)
    val ListCardAlternative = ComposeColor(0xFF514F50)
    val TextPrimaryLight = ComposeColor(0xFF1D1C1E)
    val TextPrimaryDark = ComposeColor(0xFFEDEDED)
    val DividerLightAlpha = ComposeColor(0x0D000000)
    val DividerDarkAlpha = ComposeColor(0x0DFFFFFF)
    val TransactionReceived = ComposeColor(0xFF2DC24E)
    val IconGray = ComposeColor(0xFF6F6F75)
    val ButtonDisabled = ComposeColor(0xFFD0D0D0)
    val ButtonDisabledText = ComposeColor(0xFF6F6F70)
}
