package org.bitcoinppl.cove.ui.theme

import androidx.compose.material3.Typography
import androidx.compose.ui.text.PlatformTextStyle
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.LineHeightStyle
import androidx.compose.ui.unit.sp

// tight text styling to match iOS spacing behavior
private val tightPlatformStyle = PlatformTextStyle(includeFontPadding = false)
private val tightLineHeightStyle =
    LineHeightStyle(
        alignment = LineHeightStyle.Alignment.Center,
        trim = LineHeightStyle.Trim.Both,
    )

// Complete Material 3 typography scale with iOS-matching tight spacing
val Typography =
    Typography(
        // Display styles - large, prominent text
        displayLarge =
            TextStyle(
                fontFamily = FontFamily.Default,
                fontWeight = FontWeight.Normal,
                fontSize = 57.sp,
                lineHeight = 57.sp,
                letterSpacing = (-0.25).sp,
                platformStyle = tightPlatformStyle,
                lineHeightStyle = tightLineHeightStyle,
            ),
        displayMedium =
            TextStyle(
                fontFamily = FontFamily.Default,
                fontWeight = FontWeight.Normal,
                fontSize = 45.sp,
                lineHeight = 45.sp,
                letterSpacing = 0.sp,
                platformStyle = tightPlatformStyle,
                lineHeightStyle = tightLineHeightStyle,
            ),
        displaySmall =
            TextStyle(
                fontFamily = FontFamily.Default,
                fontWeight = FontWeight.Normal,
                fontSize = 36.sp,
                lineHeight = 36.sp,
                letterSpacing = 0.sp,
                platformStyle = tightPlatformStyle,
                lineHeightStyle = tightLineHeightStyle,
            ),
        // Headline styles - high-emphasis text
        headlineLarge =
            TextStyle(
                fontFamily = FontFamily.Default,
                fontWeight = FontWeight.Normal,
                fontSize = 32.sp,
                lineHeight = 32.sp,
                letterSpacing = 0.sp,
                platformStyle = tightPlatformStyle,
                lineHeightStyle = tightLineHeightStyle,
            ),
        headlineMedium =
            TextStyle(
                fontFamily = FontFamily.Default,
                fontWeight = FontWeight.Normal,
                fontSize = 28.sp,
                lineHeight = 28.sp,
                letterSpacing = 0.sp,
                platformStyle = tightPlatformStyle,
                lineHeightStyle = tightLineHeightStyle,
            ),
        headlineSmall =
            TextStyle(
                fontFamily = FontFamily.Default,
                fontWeight = FontWeight.Normal,
                fontSize = 24.sp,
                lineHeight = 24.sp,
                letterSpacing = 0.sp,
                platformStyle = tightPlatformStyle,
                lineHeightStyle = tightLineHeightStyle,
            ),
        // Title styles - medium-emphasis text
        titleLarge =
            TextStyle(
                fontFamily = FontFamily.Default,
                fontWeight = FontWeight.Normal,
                fontSize = 22.sp,
                lineHeight = 22.sp,
                letterSpacing = 0.sp,
                platformStyle = tightPlatformStyle,
                lineHeightStyle = tightLineHeightStyle,
            ),
        titleMedium =
            TextStyle(
                fontFamily = FontFamily.Default,
                fontWeight = FontWeight.Medium,
                fontSize = 16.sp,
                lineHeight = 16.sp,
                letterSpacing = 0.15.sp,
                platformStyle = tightPlatformStyle,
                lineHeightStyle = tightLineHeightStyle,
            ),
        titleSmall =
            TextStyle(
                fontFamily = FontFamily.Default,
                fontWeight = FontWeight.Medium,
                fontSize = 14.sp,
                lineHeight = 14.sp,
                letterSpacing = 0.1.sp,
                platformStyle = tightPlatformStyle,
                lineHeightStyle = tightLineHeightStyle,
            ),
        // Body styles - main content text
        bodyLarge =
            TextStyle(
                fontFamily = FontFamily.Default,
                fontWeight = FontWeight.Normal,
                fontSize = 16.sp,
                lineHeight = 16.sp,
                letterSpacing = 0.5.sp,
                platformStyle = tightPlatformStyle,
                lineHeightStyle = tightLineHeightStyle,
            ),
        bodyMedium =
            TextStyle(
                fontFamily = FontFamily.Default,
                fontWeight = FontWeight.Normal,
                fontSize = 14.sp,
                lineHeight = 14.sp,
                letterSpacing = 0.25.sp,
                platformStyle = tightPlatformStyle,
                lineHeightStyle = tightLineHeightStyle,
            ),
        bodySmall =
            TextStyle(
                fontFamily = FontFamily.Default,
                fontWeight = FontWeight.Normal,
                fontSize = 12.sp,
                lineHeight = 12.sp,
                letterSpacing = 0.4.sp,
                platformStyle = tightPlatformStyle,
                lineHeightStyle = tightLineHeightStyle,
            ),
        // Label styles - UI elements, buttons
        labelLarge =
            TextStyle(
                fontFamily = FontFamily.Default,
                fontWeight = FontWeight.Medium,
                fontSize = 14.sp,
                lineHeight = 14.sp,
                letterSpacing = 0.1.sp,
                platformStyle = tightPlatformStyle,
                lineHeightStyle = tightLineHeightStyle,
            ),
        labelMedium =
            TextStyle(
                fontFamily = FontFamily.Default,
                fontWeight = FontWeight.Medium,
                fontSize = 12.sp,
                lineHeight = 12.sp,
                letterSpacing = 0.5.sp,
                platformStyle = tightPlatformStyle,
                lineHeightStyle = tightLineHeightStyle,
            ),
        labelSmall =
            TextStyle(
                fontFamily = FontFamily.Default,
                fontWeight = FontWeight.Medium,
                fontSize = 11.sp,
                lineHeight = 11.sp,
                letterSpacing = 0.5.sp,
                platformStyle = tightPlatformStyle,
                lineHeightStyle = tightLineHeightStyle,
            ),
    )
