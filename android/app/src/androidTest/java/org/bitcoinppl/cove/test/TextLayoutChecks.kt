package org.bitcoinppl.cove.test

import androidx.compose.ui.semantics.SemanticsActions
import androidx.compose.ui.semantics.SemanticsNode
import androidx.compose.ui.semantics.getOrNull
import androidx.compose.ui.text.TextLayoutResult

// a text's box is its widest line rounded up to a whole pixel
private const val LINE_WIDTH_TOLERANCE_PX = 1f

/**
 * Why the text's layout doesn't show whole, or null when it does: no line ellipsized, no line
 * past the text's height or line limit, and no line wider than the text
 *
 * A text node reports its layout through this action, as the accessibility service reads it.
 * Compose lays that copy out again at the text's maximum width constraint rather than the width
 * it took, so its paragraph width is the parent's and `didOverflowWidth` holds for any text
 * narrower than its parent. Line breaks and heights match the drawn text, since a text only
 * takes less than its maximum width when it fits on its lines without wrapping, so the width
 * check measures each line instead. A node that reports no layout fails too, so a change that
 * hides the layout can't turn the check off
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
        else -> null
    }
}
