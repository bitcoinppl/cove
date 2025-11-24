package org.bitcoinppl.cove.views

import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.material3.LocalTextStyle
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.LocalFontFamilyResolver
import androidx.compose.ui.text.TextLayoutResult
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontStyle
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextDecoration
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.Density
import androidx.compose.ui.unit.TextUnit
import androidx.compose.ui.unit.sp
import kotlin.math.floor
import kotlin.math.log10

/**
 * Text composable that automatically resizes to fit available width
 * Mimics iOS minimumScaleFactor behavior
 *
 * @param text The text to display
 * @param modifier Modifier for the text
 * @param color Text color
 * @param maxFontSize Maximum font size (will shrink from this)
 * @param minimumScaleFactor Minimum scale factor (0.0 to 1.0). Text will shrink to this percentage
 * @param fontWeight Font weight
 * @param fontStyle Font style
 * @param fontFamily Font family
 * @param textAlign Text alignment
 * @param textDecoration Text decoration
 * @param style Additional text style
 * @param onTextLayout Callback for text layout
 */
@Composable
fun AutoSizeText(
    text: String,
    modifier: Modifier = Modifier,
    color: Color = Color.Unspecified,
    maxFontSize: TextUnit = 14.sp,
    minimumScaleFactor: Float = 0.9f,
    fontWeight: FontWeight? = null,
    fontStyle: FontStyle? = null,
    fontFamily: FontFamily? = null,
    textAlign: TextAlign? = null,
    textDecoration: TextDecoration? = null,
    style: TextStyle = LocalTextStyle.current,
    onTextLayout: ((TextLayoutResult) -> Unit)? = null,
) {
    val density = LocalDensity.current
    val fontFamilyResolver = LocalFontFamilyResolver.current
    val minFontSize = maxFontSize * minimumScaleFactor

    BoxWithConstraints(modifier = modifier) {
        val maxWidthPx = with(density) { maxWidth.toPx() }

        // calculate optimal font size
        val fontSize = remember(text, maxWidthPx, maxFontSize, minFontSize) {
            calculateOptimalFontSize(
                text = text,
                maxFontSize = maxFontSize,
                minFontSize = minFontSize,
                maxWidthPx = maxWidthPx,
                style = style,
                fontWeight = fontWeight,
                fontStyle = fontStyle,
                fontFamily = fontFamily,
                density = density,
                fontFamilyResolver = fontFamilyResolver,
            )
        }

        Text(
            text = text,
            color = color,
            fontSize = fontSize,
            fontWeight = fontWeight,
            fontStyle = fontStyle,
            fontFamily = fontFamily,
            textAlign = textAlign,
            textDecoration = textDecoration,
            style = style,
            maxLines = 1,
            softWrap = false,
            overflow = TextOverflow.Ellipsis,
            onTextLayout = onTextLayout,
        )
    }
}

/**
 * Balance text that uses digit-based font size reduction
 * Matches iOS implementation: base font reduces by 2sp per digit, minimum 20sp
 *
 * @param text The balance text to display
 * @param modifier Modifier for the text
 * @param color Text color
 * @param baseFontSize Base font size (default 34.sp to match iOS)
 * @param minFontSize Minimum font size (default 20.sp to match iOS)
 * @param fontWeight Font weight
 * @param style Additional text style
 * @param onTextLayout Callback for text layout
 */
@Composable
fun BalanceAutoSizeText(
    text: String,
    modifier: Modifier = Modifier,
    color: Color = Color.Unspecified,
    baseFontSize: TextUnit = 34.sp,
    minFontSize: TextUnit = 20.sp,
    fontWeight: FontWeight? = null,
    style: TextStyle = LocalTextStyle.current,
    onTextLayout: ((TextLayoutResult) -> Unit)? = null,
) {
    val density = LocalDensity.current
    val fontFamilyResolver = LocalFontFamilyResolver.current

    // extract numeric value to count digits (matches iOS algorithm)
    val digits = remember(text) {
        val numericText = text.replace(Regex("[^0-9.]"), "")
        val number = numericText.toDoubleOrNull() ?: 0.0
        if (number > 0) {
            floor(log10(number)).toInt() + 1
        } else {
            1
        }
    }

    // calculate font size based on digits: max(baseFontSize - (digits - 1) * 2, minFontSize)
    val digitBasedFontSize = remember(digits, baseFontSize, minFontSize) {
        val reduction = (digits - 1) * 2
        val calculated = baseFontSize.value - reduction
        maxOf(calculated, minFontSize.value).sp
    }

    BoxWithConstraints(modifier = modifier) {
        val maxWidthPx = with(density) { maxWidth.toPx() }

        // ensure the calculated size still fits, otherwise shrink further
        val finalFontSize = remember(text, maxWidthPx, digitBasedFontSize, minFontSize) {
            calculateOptimalFontSize(
                text = text,
                maxFontSize = digitBasedFontSize,
                minFontSize = minFontSize,
                maxWidthPx = maxWidthPx,
                style = style,
                fontWeight = fontWeight,
                fontStyle = null,
                fontFamily = null,
                density = density,
                fontFamilyResolver = fontFamilyResolver,
            )
        }

        Text(
            text = text,
            color = color,
            fontSize = finalFontSize,
            fontWeight = fontWeight,
            style = style,
            maxLines = 1,
            softWrap = false,
            overflow = TextOverflow.Ellipsis,
            onTextLayout = onTextLayout,
        )
    }
}

/**
 * Calculate optimal font size using binary search
 */
private fun calculateOptimalFontSize(
    text: String,
    maxFontSize: TextUnit,
    minFontSize: TextUnit,
    maxWidthPx: Float,
    style: TextStyle,
    fontWeight: FontWeight?,
    fontStyle: FontStyle?,
    fontFamily: FontFamily?,
    density: Density,
    fontFamilyResolver: FontFamily.Resolver,
): TextUnit {
    var low = minFontSize.value
    var high = maxFontSize.value
    var optimalSize = minFontSize.value

    // binary search for optimal font size
    while (low <= high) {
        val mid = (low + high) / 2f
        val testStyle = style.copy(
            fontSize = mid.sp,
            fontWeight = fontWeight,
            fontStyle = fontStyle,
            fontFamily = fontFamily,
        )

        // measure text width at this font size
        val paragraph = androidx.compose.ui.text.Paragraph(
            text = text,
            style = testStyle,
            constraints = androidx.compose.ui.unit.Constraints(maxWidth = Int.MAX_VALUE),
            density = density,
            fontFamilyResolver = fontFamilyResolver,
        )

        if (paragraph.minIntrinsicWidth <= maxWidthPx) {
            // text fits, try larger size
            optimalSize = mid
            low = mid + 0.5f
        } else {
            // text doesn't fit, try smaller size
            high = mid - 0.5f
        }
    }

    return optimalSize.sp
}
