package org.bitcoinppl.cove.test

import androidx.compose.ui.semantics.SemanticsActions
import androidx.compose.ui.semantics.SemanticsNode
import androidx.compose.ui.semantics.getOrNull
import androidx.compose.ui.text.ParagraphIntrinsics
import androidx.compose.ui.text.TextLayoutResult

// a text's box is its widest line rounded up to a whole pixel
private const val LINE_WIDTH_TOLERANCE_PX = 1f

/**
 * Why the text's layout doesn't show whole, or null when it does: no line ellipsized, no line
 * past the text's height or line limit, no line wider than the text, and no word broken across
 * lines that would fit on a line as wide as its window
 *
 * A text node reports its layout through this action, as the accessibility service reads it.
 * Compose lays that copy out again at the text's maximum width constraint rather than the width
 * it took, so its paragraph width is the parent's and `didOverflowWidth` holds for any text
 * narrower than its parent. Line breaks and heights match the drawn text, since a text only
 * takes less than its maximum width when it fits on its lines without wrapping, so the width
 * check measures each line instead. A node that reports no layout fails too, so a change that
 * hides the layout can't turn the check off
 *
 * Compose adds no hyphen where it breaks a word, so a word wider than the room its container
 * gives draws as "Hard" over "ware". Without hyphenation a word only breaks when it is wider
 * than its own line, so the word is measured against its window instead: a container too narrow
 * for a word the window could hold fails, while a token wider than the window, such as an xprv
 * or an address, has to break somewhere and passes
 *
 * Read it on the main thread
 */
fun SemanticsNode.textLayoutProblem(): String? {
    val action = config.getOrNull(SemanticsActions.GetTextLayoutResult)?.action ?: return "its text reports no layout"
    val layouts = mutableListOf<TextLayoutResult>()
    if (!action(layouts)) return "its text has no layout yet"

    val layout = layouts.singleOrNull() ?: return "its text reports ${layouts.size} layouts"
    val lines = 0 until layout.lineCount
    val ellipsized = lines.filter(layout::isLineEllipsized)
    val widest = lines.maxOfOrNull { layout.getLineRight(it) - layout.getLineLeft(it) } ?: 0f

    return when {
        ellipsized.isNotEmpty() -> "line ${ellipsized.first() + 1} of ${layout.lineCount} is ellipsized"
        layout.multiParagraph.didExceedMaxLines -> "it needs more than its ${layout.lineCount} line limit"
        layout.didOverflowHeight -> "it needs ${layout.multiParagraph.height}px but gets ${layout.size.height}px"
        widest > layout.size.width + LINE_WIDTH_TOLERANCE_PX -> "a line needs ${widest}px wide but gets ${layout.size.width}px"
        else -> layout.brokenWordProblem(windowWidth = generateSequence(this) { it.parent }.last().size.width.toFloat())
    }
}

/**
 * The first word split across two lines that would fit on one line [windowWidth] wide, or null
 *
 * A word is a run of letters and digits, so a break at a space, hyphen, slash or dot is a word
 * break and passes. The word's width is the sum of its drawn parts on each line, measured from
 * the characters' boxes
 */
private fun TextLayoutResult.brokenWordProblem(windowWidth: Float): String? {
    val text = layoutInput.text.text

    for (line in 0 until lineCount - 1) {
        val breakAt = getLineEnd(line)
        val isInsideWord = breakAt in 1 until text.length && text[breakAt - 1].isLetterOrDigit() && text[breakAt].isLetterOrDigit()
        if (!isInsideWord) continue

        var start = breakAt - 1
        while (start > 0 && text[start - 1].isLetterOrDigit()) start--
        var end = breakAt
        while (end < text.length && text[end].isLetterOrDigit()) end++

        val word = text.substring(start, end)
        val wordWidth = wordWidth(start, end)
        if (wordWidth <= windowWidth + LINE_WIDTH_TOLERANCE_PX) {
            return "\"$word\" breaks after \"${text.substring(start, breakAt)}\" in a ${multiParagraph.width}px line, " +
                "though its ${wordWidth}px fit its ${windowWidth}px window"
        }
    }

    return null
}

// a word split across lines draws one part per line, so its width on one line is their sum
private fun TextLayoutResult.wordWidth(
    start: Int,
    end: Int,
): Float =
    (start until end)
        .groupBy(::getLineForOffset)
        .values
        .map { offsets -> offsets.map(::getBoundingBox) }
        .sumOf { boxes -> (boxes.maxOf { it.right } - boxes.minOf { it.left }).toDouble() }
        .toFloat()

/**
 * Why the text draws smaller than its style's font size, or null when it draws at that size
 *
 * An autosizing text shrinks to fit rather than wrapping, so it lays out whole however small it
 * gets, and [textLayoutProblem] passes it. Its reported layout keeps the style it was given while
 * its paragraph holds the size it chose, so a paragraph narrower than the same text measured at
 * the style's size has shrunk. Read it on the main thread
 */
fun SemanticsNode.shrunkTextProblem(): String? {
    val action = config.getOrNull(SemanticsActions.GetTextLayoutResult)?.action ?: return "its text reports no layout"
    val layouts = mutableListOf<TextLayoutResult>()
    if (!action(layouts)) return "its text has no layout yet"

    val layout = layouts.singleOrNull() ?: return "its text reports ${layouts.size} layouts"
    val input = layout.layoutInput
    val fullWidth =
        ParagraphIntrinsics(
            text = input.text.text,
            style = input.style,
            density = input.density,
            fontFamilyResolver = input.fontFamilyResolver,
        ).maxIntrinsicWidth
    val drawnWidth = layout.multiParagraph.intrinsics.maxIntrinsicWidth

    return if (drawnWidth < fullWidth - LINE_WIDTH_TOLERANCE_PX) {
        "it draws ${drawnWidth}px wide on one line, shrunk from the ${fullWidth}px its ${input.style.fontSize} style needs"
    } else {
        null
    }
}
