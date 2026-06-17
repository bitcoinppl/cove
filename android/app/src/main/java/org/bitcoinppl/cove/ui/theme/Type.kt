package org.bitcoinppl.cove.ui.theme

import androidx.compose.material3.Typography
import androidx.compose.ui.text.PlatformTextStyle
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.LineHeightStyle
import androidx.compose.ui.unit.sp

// tight text styling to match iOS spacing behavior
internal val tightPlatformStyle = PlatformTextStyle(includeFontPadding = false)
internal val tightLineHeightStyle =
    LineHeightStyle(
        alignment = LineHeightStyle.Alignment.Center,
        trim = LineHeightStyle.Trim.Both,
    )

// SwiftUI default text style sizes with iOS-matching tight spacing
val Typography =
    Typography(
        // display styles are kept for oversized one-off UI states
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
        // headline styles map to largeTitle, title, and title2
        headlineLarge =
            TextStyle(
                fontFamily = FontFamily.Default,
                fontWeight = FontWeight.Normal,
                fontSize = 34.sp,
                lineHeight = 34.sp,
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
                fontSize = 22.sp,
                lineHeight = 22.sp,
                letterSpacing = 0.sp,
                platformStyle = tightPlatformStyle,
                lineHeightStyle = tightLineHeightStyle,
            ),
        // title styles map to title2, headline, and subheadline
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
                fontSize = 17.sp,
                lineHeight = 17.sp,
                letterSpacing = 0.sp,
                platformStyle = tightPlatformStyle,
                lineHeightStyle = tightLineHeightStyle,
            ),
        titleSmall =
            TextStyle(
                fontFamily = FontFamily.Default,
                fontWeight = FontWeight.Medium,
                fontSize = 15.sp,
                lineHeight = 15.sp,
                letterSpacing = 0.sp,
                platformStyle = tightPlatformStyle,
                lineHeightStyle = tightLineHeightStyle,
            ),
        // body styles map to body, subheadline, and footnote
        bodyLarge =
            TextStyle(
                fontFamily = FontFamily.Default,
                fontWeight = FontWeight.Normal,
                fontSize = 17.sp,
                lineHeight = 17.sp,
                letterSpacing = 0.sp,
                platformStyle = tightPlatformStyle,
                lineHeightStyle = tightLineHeightStyle,
            ),
        bodyMedium =
            TextStyle(
                fontFamily = FontFamily.Default,
                fontWeight = FontWeight.Normal,
                fontSize = 15.sp,
                lineHeight = 15.sp,
                letterSpacing = 0.sp,
                platformStyle = tightPlatformStyle,
                lineHeightStyle = tightLineHeightStyle,
            ),
        bodySmall =
            TextStyle(
                fontFamily = FontFamily.Default,
                fontWeight = FontWeight.Normal,
                fontSize = 13.sp,
                lineHeight = 13.sp,
                letterSpacing = 0.sp,
                platformStyle = tightPlatformStyle,
                lineHeightStyle = tightLineHeightStyle,
            ),
        // label styles map to headline, footnote, and caption2
        labelLarge =
            TextStyle(
                fontFamily = FontFamily.Default,
                fontWeight = FontWeight.Medium,
                fontSize = 17.sp,
                lineHeight = 17.sp,
                letterSpacing = 0.sp,
                platformStyle = tightPlatformStyle,
                lineHeightStyle = tightLineHeightStyle,
            ),
        labelMedium =
            TextStyle(
                fontFamily = FontFamily.Default,
                fontWeight = FontWeight.Medium,
                fontSize = 13.sp,
                lineHeight = 13.sp,
                letterSpacing = 0.sp,
                platformStyle = tightPlatformStyle,
                lineHeightStyle = tightLineHeightStyle,
            ),
        labelSmall =
            TextStyle(
                fontFamily = FontFamily.Default,
                fontWeight = FontWeight.Medium,
                fontSize = 11.sp,
                lineHeight = 11.sp,
                letterSpacing = 0.sp,
                platformStyle = tightPlatformStyle,
                lineHeightStyle = tightLineHeightStyle,
            ),
    )

// extension to match iOS title3 (20pt)
val Typography.title3: TextStyle
    get() =
        TextStyle(
            fontFamily = FontFamily.Default,
            fontWeight = FontWeight.Normal,
            fontSize = 20.sp,
            lineHeight = 20.sp,
            letterSpacing = 0.sp,
            platformStyle = tightPlatformStyle,
            lineHeightStyle = tightLineHeightStyle,
        )

// extension to match iOS callout (16pt)
val Typography.callout: TextStyle
    get() =
        TextStyle(
            fontFamily = FontFamily.Default,
            fontWeight = FontWeight.Normal,
            fontSize = 16.sp,
            lineHeight = 16.sp,
            letterSpacing = 0.sp,
            platformStyle = tightPlatformStyle,
            lineHeightStyle = tightLineHeightStyle,
        )

// extension to match iOS caption (12pt)
val Typography.caption: TextStyle
    get() =
        TextStyle(
            fontFamily = FontFamily.Default,
            fontWeight = FontWeight.Normal,
            fontSize = 12.sp,
            lineHeight = 12.sp,
            letterSpacing = 0.sp,
            platformStyle = tightPlatformStyle,
            lineHeightStyle = tightLineHeightStyle,
        )
