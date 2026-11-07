package org.bitcoinppl.cove.views

import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.layout.Layout
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.constrainHeight
import androidx.compose.ui.unit.dp

/**
 * Lays its children side by side in equal widths when each fits its share without wrapping, and
 * stacks them at full width otherwise
 *
 * Each child's max intrinsic width is the width it needs to draw its label on one line at its
 * full size, so a label that would break inside a word, or shrink, beside the others gets the
 * full width instead. Needs a bounded width, as a screen column gives it
 */
@Composable
fun SideBySideOrStacked(
    modifier: Modifier = Modifier,
    spacing: Dp = 16.dp,
    content: @Composable () -> Unit,
) {
    Layout(content = content, modifier = modifier) { measurables, constraints ->
        val spacingPx = spacing.roundToPx()
        val width = constraints.maxWidth
        val gaps = spacingPx * (measurables.size - 1).coerceAtLeast(0)
        val share = if (measurables.isEmpty()) 0 else ((width - gaps) / measurables.size).coerceAtLeast(0)
        val fitsSideBySide = measurables.all { it.maxIntrinsicWidth(constraints.maxHeight) <= share }

        if (fitsSideBySide) {
            val placeables = measurables.map { it.measure(constraints.copy(minWidth = share, maxWidth = share, minHeight = 0)) }
            val height = placeables.maxOfOrNull { it.height } ?: 0

            layout(width, constraints.constrainHeight(height)) {
                placeables.forEachIndexed { index, placeable ->
                    placeable.placeRelative(index * (share + spacingPx), 0)
                }
            }
        } else {
            val placeables = measurables.map { it.measure(constraints.copy(minWidth = width, minHeight = 0)) }
            val height = placeables.sumOf { it.height } + gaps

            layout(width, constraints.constrainHeight(height)) {
                var y = 0
                placeables.forEach { placeable ->
                    placeable.placeRelative(0, y)
                    y += placeable.height + spacingPx
                }
            }
        }
    }
}
