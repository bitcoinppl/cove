package org.bitcoinppl.cove.views

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.runtime.Stable
import androidx.compose.ui.unit.Density
import androidx.compose.ui.unit.Dp
import kotlin.math.roundToInt

/**
 * Places children like [Arrangement.SpaceBetween], but never closer together than [minSpacing]
 *
 * A screen that pins one section to the top and another to the bottom with SpaceBetween inside a
 * scrolling `heightIn(min = maxHeight)` column loses every gap once its content outgrows the
 * viewport, as it does on a small screen or at a large display or font size, so the sections
 * touch. A column measures an arrangement's spacing into its own height, so with this one the
 * content scrolls by the extra [minSpacing] instead
 */
@Stable
fun Arrangement.spaceBetweenAtLeast(minSpacing: Dp): Arrangement.Vertical = SpaceBetweenAtLeast(minSpacing)

private data class SpaceBetweenAtLeast(
    override val spacing: Dp,
) : Arrangement.Vertical {
    override fun Density.arrange(
        totalSize: Int,
        sizes: IntArray,
        outPositions: IntArray,
    ) {
        // the column's height already holds the minimum spacing, so the free space shared out
        // here never leaves a gap below it
        val gapCount = sizes.size - 1
        val freeSpace = totalSize - sizes.sum()
        val gap = if (gapCount > 0) freeSpace.toFloat() / gapCount else 0f

        var position = 0f
        sizes.forEachIndexed { index, size ->
            outPositions[index] = position.roundToInt()
            position += size + gap
        }
    }
}
