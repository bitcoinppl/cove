package org.bitcoinppl.cove.views

import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material3.LocalTextStyle
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.focus.onFocusChanged
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.LocalFontFamilyResolver
import androidx.compose.ui.text.TextLayoutResult
import androidx.compose.ui.text.TextRange
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontStyle
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.input.TextFieldValue
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextDecoration
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.Density
import androidx.compose.ui.unit.TextUnit
import androidx.compose.ui.unit.sp

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

        // calculate optimal font size with all dependencies
        val fontSize =
            remember(
                text,
                maxWidthPx,
                maxFontSize,
                minFontSize,
                style,
                fontWeight,
                fontStyle,
                fontFamily,
            ) {
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
 * Matches iOS implementation: base font reduces by 2sp per digit
 *
 * @param text The balance text to display
 * @param modifier Modifier for the text
 * @param color Text color
 * @param baseFontSize Base font size (default 34.sp to match iOS)
 * @param minimumScaleFactor Minimum scale factor (0.0 to 1.0), text shrinks to this percentage of baseFontSize
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
    minimumScaleFactor: Float = 0.5f,
    fontWeight: FontWeight? = null,
    textAlign: TextAlign? = null,
    style: TextStyle = LocalTextStyle.current,
    onTextLayout: ((TextLayoutResult) -> Unit)? = null,
) {
    val density = LocalDensity.current
    val fontFamilyResolver = LocalFontFamilyResolver.current
    val minFontSize = baseFontSize * minimumScaleFactor

    BoxWithConstraints(modifier = modifier) {
        val maxWidthPx = with(density) { maxWidth.toPx() }

        // let binary search find optimal size from baseFontSize
        val finalFontSize =
            remember(
                text,
                maxWidthPx,
                baseFontSize,
                minFontSize,
                style,
                fontWeight,
            ) {
                calculateOptimalFontSize(
                    text = text,
                    maxFontSize = baseFontSize,
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
            modifier = Modifier.fillMaxWidth(),
            color = color,
            fontSize = finalFontSize,
            fontWeight = fontWeight,
            textAlign = textAlign,
            style = style,
            maxLines = 1,
            softWrap = false,
            overflow = TextOverflow.Ellipsis,
            onTextLayout = onTextLayout,
        )
    }
}

/**
 * Editable text field that automatically resizes to fit available width
 * Mimics iOS TextField with minimumScaleFactor behavior
 *
 * Uses TextFieldValue internally to properly manage cursor position when
 * the text is modified externally (e.g., formatting with commas).
 *
 * @param value The current text value
 * @param onValueChange Callback when text changes
 * @param modifier Modifier for the text field
 * @param maxFontSize Maximum font size (will shrink from this)
 * @param minimumScaleFactor Minimum scale factor (0.0 to 1.0). Text will shrink to this percentage
 * @param color Text color
 * @param fontWeight Font weight
 * @param textAlign Text alignment
 * @param onTextWidthChanged Callback with measured text width in Dp (for positioning related elements)
 */
@Composable
fun AutoSizeTextField(
    value: String,
    onValueChange: (String) -> Unit,
    modifier: Modifier = Modifier,
    maxFontSize: TextUnit = 48.sp,
    minimumScaleFactor: Float = 0.01f,
    color: Color = Color.Unspecified,
    fontWeight: FontWeight? = null,
    textAlign: TextAlign? = null,
    onTextWidthChanged: ((androidx.compose.ui.unit.Dp) -> Unit)? = null,
    onFocusChanged: ((Boolean) -> Unit)? = null,
) {
    val density = LocalDensity.current
    val fontFamilyResolver = LocalFontFamilyResolver.current
    val minFontSize = maxFontSize * minimumScaleFactor
    val style = LocalTextStyle.current

    // use TextFieldValue internally to control cursor position
    // track the last external value to detect when it changes
    var lastExternalValue by remember { mutableStateOf(value) }
    var textFieldValue by remember {
        mutableStateOf(TextFieldValue(value, TextRange(value.length)))
    }

    // sync from external value changes (e.g., formatting), keeping cursor at end
    // this runs during composition for immediate effect, not in a side effect
    if (value != lastExternalValue) {
        lastExternalValue = value
        if (textFieldValue.text != value) {
            textFieldValue = TextFieldValue(value, TextRange(value.length))
        }
    }

    BoxWithConstraints(modifier = modifier) {
        val maxWidthPx = with(density) { maxWidth.toPx() }

        val (fontSize, textWidthPx) =
            remember(
                textFieldValue.text,
                maxWidthPx,
                maxFontSize,
                minFontSize,
                style,
                fontWeight,
            ) {
                calculateOptimalFontSizeAndWidth(
                    text = textFieldValue.text,
                    maxFontSize = maxFontSize,
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

        // report text width in Dp
        val textWidthDp = with(density) { textWidthPx.toDp() }
        onTextWidthChanged?.invoke(textWidthDp)

        BasicTextField(
            value = textFieldValue,
            onValueChange = { newValue ->
                textFieldValue = newValue
                if (newValue.text != value) {
                    onValueChange(newValue.text)
                }
            },
            textStyle =
                TextStyle(
                    color = color,
                    fontSize = fontSize,
                    fontWeight = fontWeight,
                    textAlign = textAlign ?: TextAlign.Unspecified,
                ),
            singleLine = true,
            keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Decimal),
            cursorBrush = SolidColor(color),
            modifier =
                Modifier
                    .fillMaxWidth()
                    .onFocusChanged { focusState ->
                        onFocusChanged?.invoke(focusState.isFocused)
                    },
        )
    }
}

/**
 * Calculate optimal font size using binary search with 0.1sp precision
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
    // guard against invalid width - return max font size if we can't measure
    if (maxWidthPx <= 0 || maxWidthPx == Float.MAX_VALUE) {
        return maxFontSize
    }

    var low = minFontSize.value
    var high = maxFontSize.value
    var optimalSize = maxFontSize.value // start with max, shrink only if needed

    // binary search for optimal font size with 0.1sp precision
    while (high - low > 0.1f) {
        val mid = (low + high) / 2f
        val testStyle =
            style.copy(
                fontSize = mid.sp,
                fontWeight = fontWeight,
                fontStyle = fontStyle,
                fontFamily = fontFamily,
            )

        // measure text width at this font size
        val paragraph =
            androidx.compose.ui.text.Paragraph(
                text = text,
                style = testStyle,
                constraints =
                    androidx.compose.ui.unit
                        .Constraints(maxWidth = Int.MAX_VALUE),
                density = density,
                fontFamilyResolver = fontFamilyResolver,
            )

        if (paragraph.minIntrinsicWidth <= maxWidthPx) {
            // text fits, try larger size
            optimalSize = mid
            low = mid + 0.1f
        } else {
            // text doesn't fit, try smaller size
            high = mid - 0.1f
        }
    }

    return optimalSize.sp
}

/**
 * Calculate optimal font size and text width using binary search with 0.1sp precision
 * Returns a Pair of (fontSize, textWidthPx)
 */
private fun calculateOptimalFontSizeAndWidth(
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
): Pair<TextUnit, Float> {
    // guard against invalid width - return max font size if we can't measure
    if (maxWidthPx <= 0 || maxWidthPx == Float.MAX_VALUE) {
        return Pair(maxFontSize, 0f)
    }

    var low = minFontSize.value
    var high = maxFontSize.value
    var optimalSize = maxFontSize.value // start with max, shrink only if needed
    var textWidth = 0f

    // binary search for optimal font size with 0.1sp precision
    while (high - low > 0.1f) {
        val mid = (low + high) / 2f
        val testStyle =
            style.copy(
                fontSize = mid.sp,
                fontWeight = fontWeight,
                fontStyle = fontStyle,
                fontFamily = fontFamily,
            )

        // measure text width at this font size
        val paragraph =
            androidx.compose.ui.text.Paragraph(
                text = text,
                style = testStyle,
                constraints =
                    androidx.compose.ui.unit
                        .Constraints(maxWidth = Int.MAX_VALUE),
                density = density,
                fontFamilyResolver = fontFamilyResolver,
            )

        if (paragraph.minIntrinsicWidth <= maxWidthPx) {
            // text fits, try larger size
            optimalSize = mid
            textWidth = paragraph.minIntrinsicWidth
            low = mid + 0.1f
        } else {
            // text doesn't fit, try smaller size
            high = mid - 0.1f
        }
    }

    // final measurement at optimal size to get accurate width
    val finalStyle =
        style.copy(
            fontSize = optimalSize.sp,
            fontWeight = fontWeight,
            fontStyle = fontStyle,
            fontFamily = fontFamily,
        )
    val finalParagraph =
        androidx.compose.ui.text.Paragraph(
            text = text,
            style = finalStyle,
            constraints =
                androidx.compose.ui.unit
                    .Constraints(maxWidth = Int.MAX_VALUE),
            density = density,
            fontFamilyResolver = fontFamilyResolver,
        )
    textWidth = finalParagraph.minIntrinsicWidth

    return Pair(optimalSize.sp, textWidth)
}
