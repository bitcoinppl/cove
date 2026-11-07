package org.bitcoinppl.cove.test

import android.graphics.Bitmap
import android.util.Log
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Canvas
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.graphics.ImageBitmapConfig
import androidx.compose.ui.graphics.asAndroidBitmap
import androidx.compose.ui.graphics.drawscope.CanvasDrawScope
import androidx.compose.ui.graphics.isSpecified
import androidx.compose.ui.graphics.toArgb
import androidx.compose.ui.semantics.SemanticsActions
import androidx.compose.ui.semantics.SemanticsNode
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.semantics.getOrNull
import androidx.compose.ui.test.SemanticsNodeInteraction
import androidx.compose.ui.test.captureToImage
import androidx.compose.ui.text.TextLayoutResult
import androidx.compose.ui.text.drawText
import androidx.compose.ui.unit.Density
import androidx.compose.ui.unit.IntOffset
import androidx.compose.ui.unit.LayoutDirection
import androidx.core.graphics.ColorUtils
import androidx.test.platform.app.InstrumentationRegistry
import org.junit.Assert.assertTrue
import kotlin.math.abs
import kotlin.math.ceil
import kotlin.math.floor
import kotlin.math.max
import kotlin.math.roundToInt

/**
 * How closely a capture shows a text as its own layout draws it
 *
 * The reference is the text's [TextLayoutResult] drawn offscreen as a coverage mask, then blended
 * from the text's own colour over the background the capture shows beside each glyph. Only pixels
 * the reference covers almost fully count as ink, since glyph edges rasterise a little differently
 * on screen and offscreen. Contrast comes from the captured ink itself, so translucency from any
 * layer above the text counts, not only the colour the text declares
 */
class TextReferenceMatch(
    val label: String,
    /** Pixels the reference's glyphs cover with at least [CORE_INK_COVERAGE], before any colour alpha */
    val inkPixels: Int,
    /** Ink pixels whose captured colour is far from the reference blend, or outside the capture */
    val mismatchedInk: Int,
    /**
     * Ink pixels whose captured colour, taken back to full glyph coverage, falls short of
     * [minContrast] against the background beside them
     */
    val faintInk: Int,
    /** The lowest captured text-to-background contrast over the ink pixels */
    val lowestContrast: Double,
    val minContrast: Double,
    /** The shift from the laid-out position that matched best */
    val offset: IntOffset,
) {
    val mismatchFraction: Double get() = if (inkPixels == 0) 1.0 else mismatchedInk.toDouble() / inkPixels
    val faintFraction: Double get() = if (inkPixels == 0) 1.0 else faintInk.toDouble() / inkPixels

    /** Whether the capture shows the whole text, in its colour and readable against its background */
    val matches: Boolean
        get() = inkPixels > 0 && mismatchFraction <= MAX_MISMATCHED_INK && faintFraction <= MAX_FAINT_INK

    override fun toString(): String =
        "\"$label\": ${"%.1f".format(mismatchFraction * 100)}% of $inkPixels ink pixels off the reference, " +
            "${"%.1f".format(faintFraction * 100)}% under ${minContrast}:1 (lowest ${"%.2f".format(lowestContrast)}:1), " +
            "best at offset $offset"

    companion object {
        /** Coverage from which a reference pixel counts as glyph ink rather than an anti-aliased edge */
        const val CORE_INK_COVERAGE = 0.75f

        /**
         * Share of ink pixels allowed off the reference, for rasteriser differences at glyph edges
         *
         * Covering or clipping a few characters of even the longest dialog line moves several
         * percent of its ink, well past this
         */
        const val MAX_MISMATCHED_INK = 0.02

        /** Share of ink pixels allowed under the contrast floor, for background estimates at edges */
        const val MAX_FAINT_INK = 0.01
    }
}

private const val TEXT_REFERENCE_TAG = "TextReferenceMatch"

// the exact message compose's window capture fails with when PixelCopy does not answer in time
private const val PIXEL_COPY_TIMEOUT = "Failed waiting for PixelCopy!"
private const val MAX_CAPTURE_ATTEMPTS = 3

/**
 * Capture [this] node like [captureToImage], trying again when PixelCopy times out
 *
 * Compose gives PixelCopy a fixed time to answer, which a busy emulator now and then misses. That
 * timeout says nothing about what the screen draws, so it is retried after the app goes idle, up
 * to [MAX_CAPTURE_ATTEMPTS] times; any other failure is thrown at once
 */
fun SemanticsNodeInteraction.captureToImageWithRetry(): Bitmap {
    for (attempt in 1 until MAX_CAPTURE_ATTEMPTS) {
        try {
            return captureToImage().asAndroidBitmap()
        } catch (error: AssertionError) {
            if (error.message != PIXEL_COPY_TIMEOUT) throw error
            Log.w(TEXT_REFERENCE_TAG, "capture retry: PixelCopy timed out on attempt $attempt of $MAX_CAPTURE_ATTEMPTS")
            InstrumentationRegistry.getInstrumentation().waitForIdleSync()
        }
    }

    return captureToImage().asAndroidBitmap()
}

// WCAG AA, for body text and for large text
private const val MIN_BODY_TEXT_CONTRAST = 4.5
private const val MIN_LARGE_TEXT_CONTRAST = 3.0

// WCAG large text is 18pt, or 14pt bold, which Android draws as 24 and about 18.7 scaled pixels
private const val LARGE_TEXT_SP = 24f
private const val LARGE_BOLD_TEXT_SP = 18.66f
private const val BOLD_WEIGHT = 700

// a pixel the reference leaves almost uncovered shows the background behind the text
private const val BACKGROUND_COVERAGE = 0.02f

// how far from a pixel's reference blend its captured colour may be, as a share of the
// text-to-background difference, with a floor for text close to its background
private const val MATCH_TOLERANCE_SHARE = 0.4
private const val MATCH_TOLERANCE_FLOOR = 40

// positions are whole pixels in Compose, so a small search only absorbs rounding in the capture
private const val OFFSET_SEARCH_PX = 2

/** The text layouts [this] node reports, empty for a node that draws no text */
fun SemanticsNode.textLayouts(): List<TextLayoutResult> {
    val layouts = mutableListOf<TextLayoutResult>()
    config.getOrNull(SemanticsActions.GetTextLayoutResult)?.action?.invoke(layouts)
    return layouts
}

/** The text [this] node shows, joined, for assertion messages */
fun SemanticsNode.textLabel(): String = config.getOrNull(SemanticsProperties.Text).orEmpty().joinToString { it.text }

/**
 * Compare where [node] drew in this capture with a reference rendering of its own text layout
 *
 * [captureOrigin] is the window position of the capture's top-left pixel
 */
fun Bitmap.matchTextReference(
    node: SemanticsNode,
    captureOrigin: Offset,
    density: Density,
): TextReferenceMatch {
    val layout = node.textLayouts().singleOrNull() ?: error("\"${node.textLabel()}\" reports no single text layout")
    return matchTextReference(layout, node.positionInWindow - captureOrigin, density, node.textLabel())
}

/**
 * Compare the region of this capture at [topLeft] with [layout] drawn in its own colour
 *
 * The layout is drawn whole, including lines past its own height, so text its container clips
 * shows up as missing ink. Spans are drawn in the layout's base colour
 */
fun Bitmap.matchTextReference(
    layout: TextLayoutResult,
    topLeft: Offset,
    density: Density,
    label: String,
): TextReferenceMatch {
    val style = layout.layoutInput.style
    check(style.color.isSpecified) { "\"$label\" is drawn with a brush, which the reference cannot colour" }

    val textArgb = style.color.copy(alpha = 1f).toArgb()
    val textAlpha = style.color.alpha
    val mask = layout.coverageMask(density, Offset(topLeft.x - floor(topLeft.x), topLeft.y - floor(topLeft.y)))
    val minContrast = layout.minContrast(density)
    val left = floor(topLeft.x).toInt()
    val top = floor(topLeft.y).toInt()

    // nearest offsets first, so a tie keeps the laid-out position
    val offsets =
        (-OFFSET_SEARCH_PX..OFFSET_SEARCH_PX)
            .flatMap { dy -> (-OFFSET_SEARCH_PX..OFFSET_SEARCH_PX).map { dx -> IntOffset(dx, dy) } }
            .sortedBy { abs(it.x) + abs(it.y) }

    return offsets
        .map { compareAt(mask, textArgb, textAlpha, left + it.x, top + it.y, minContrast, label, it) }
        .minBy { it.mismatchedInk }
}

/** Fail with every text in [matches] that does not match its reference */
fun assertTextsMatchTheirReferences(
    moment: String,
    matches: List<TextReferenceMatch>,
) {
    assertTrue("$moment shows no text", matches.isNotEmpty())
    matches.forEach { Log.i(TEXT_REFERENCE_TAG, "$moment: $it") }
    val failed = matches.filterNot { it.matches }
    assertTrue("$moment does not draw its text as laid out: ${failed.joinToString("; ")}", failed.isEmpty())
}

// alpha of each pixel is how much of it the text's glyphs cover
private fun TextLayoutResult.coverageMask(
    density: Density,
    subPixel: Offset,
): Bitmap {
    val width = max(size.width, ceil(multiParagraph.width).toInt()) + 1
    val height = max(size.height, ceil(multiParagraph.height).toInt()) + 1
    val image = ImageBitmap(width, height, ImageBitmapConfig.Argb8888)

    CanvasDrawScope().draw(density, LayoutDirection.Ltr, Canvas(image), Size(width.toFloat(), height.toFloat())) {
        drawText(this@coverageMask, color = Color.White, topLeft = subPixel)
    }

    return image.asAndroidBitmap()
}

private fun TextLayoutResult.minContrast(density: Density): Double {
    val style = layoutInput.style
    val fontSp = with(density) { style.fontSize.takeIf { it.isSp }?.value ?: 0f }
    val bold = (style.fontWeight?.weight ?: 0) >= BOLD_WEIGHT
    val large = fontSp >= LARGE_TEXT_SP || (bold && fontSp >= LARGE_BOLD_TEXT_SP)
    return if (large) MIN_LARGE_TEXT_CONTRAST else MIN_BODY_TEXT_CONTRAST
}

private fun Bitmap.compareAt(
    mask: Bitmap,
    textArgb: Int,
    textAlpha: Float,
    left: Int,
    top: Int,
    minContrast: Double,
    label: String,
    offset: IntOffset,
): TextReferenceMatch {
    var ink = 0
    var mismatched = 0
    var faint = 0
    var lowestContrast = Double.MAX_VALUE
    val row = IntArray(mask.width)
    val coverage = FloatArray(mask.width)

    for (y in 0 until mask.height) {
        val captureY = top + y
        mask.getPixels(row, 0, mask.width, 0, y, mask.width, 1)
        for (x in 0 until mask.width) coverage[x] = (row[x] ushr 24) / 255f
        if (coverage.none { it >= TextReferenceMatch.CORE_INK_COVERAGE }) continue

        val background = backgroundRow(captureY, left, coverage)
        for (x in 0 until mask.width) {
            val c = coverage[x]
            if (c < TextReferenceMatch.CORE_INK_COVERAGE) continue

            ink++
            val captureX = left + x
            val behind = background?.get(x)
            if (behind == null || captureX !in 0 until width || captureY !in 0 until height) {
                mismatched++
                continue
            }

            val captured = getPixel(captureX, captureY) or OPAQUE

            // what the glyph shows where it covers the pixel fully, whichever layer made it translucent
            val contrast = ColorUtils.calculateContrast(unblend(behind, captured, c), behind)
            lowestContrast = minOf(lowestContrast, contrast)
            if (contrast < minContrast) faint++

            val expected = blend(behind, textArgb, c * textAlpha)
            val tolerance =
                max(MATCH_TOLERANCE_FLOOR, (channelDistance(textArgb, behind) * textAlpha * MATCH_TOLERANCE_SHARE).roundToInt())

            if (channelDistance(captured, expected) > tolerance) mismatched++
        }
    }

    return TextReferenceMatch(
        label = label,
        inkPixels = ink,
        mismatchedInk = mismatched,
        faintInk = faint,
        lowestContrast = if (lowestContrast == Double.MAX_VALUE) 0.0 else lowestContrast,
        minContrast = minContrast,
        offset = offset,
    )
}

/**
 * The background behind each pixel of one text row, interpolated between the nearest captured
 * pixels on either side that the reference leaves uncovered, or null outside the capture
 */
private fun Bitmap.backgroundRow(
    y: Int,
    left: Int,
    coverage: FloatArray,
): Array<Int?>? {
    if (y !in 0 until height) return null

    val samples =
        coverage.indices
            .filter { coverage[it] < BACKGROUND_COVERAGE && (left + it) in 0 until width }
            .associateWith { getPixel(left + it, y) or OPAQUE }
    if (samples.isEmpty()) return null

    val sorted = samples.keys.sorted()
    return Array(coverage.size) { x ->
        val before = sorted.lastOrNull { it <= x }
        val after = sorted.firstOrNull { it >= x }
        when {
            before == null -> samples.getValue(requireNotNull(after))
            after == null || after == before -> samples.getValue(before)
            else -> blend(samples.getValue(before), samples.getValue(after), (x - before).toFloat() / (after - before))
        }
    }
}

private const val OPAQUE = 0xFF000000.toInt()

// blendARGB also blends alpha, whose float rounding turns opaque 255 into 254
private fun blend(
    from: Int,
    to: Int,
    share: Float,
): Int = ColorUtils.blendARGB(from, to, share) or OPAQUE

// the colour that, drawn over [behind] at [coverage], gives [captured], clamped to a real colour
private fun unblend(
    behind: Int,
    captured: Int,
    coverage: Float,
): Int {
    fun channel(shift: Int): Int {
        val b = behind shr shift and 0xFF
        val p = captured shr shift and 0xFF
        return (b + (p - b) / coverage).roundToInt().coerceIn(0, 255)
    }

    return OPAQUE or (channel(16) shl 16) or (channel(8) shl 8) or channel(0)
}

private fun channelDistance(
    a: Int,
    b: Int,
): Int =
    maxOf(
        abs((a shr 16 and 0xFF) - (b shr 16 and 0xFF)),
        abs((a shr 8 and 0xFF) - (b shr 8 and 0xFF)),
        abs((a and 0xFF) - (b and 0xFF)),
    )
