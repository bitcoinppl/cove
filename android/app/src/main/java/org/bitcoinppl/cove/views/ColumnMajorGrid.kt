package org.bitcoinppl.cove.views

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp

/**
 * A grid that displays items in column-major order (top-to-bottom, then left-to-right)
 *
 * For a list [1,2,3,4,5,6] with 3 columns, displays as:
 * ```
 * 1  3  5
 * 2  4  6
 * ```
 */
@Composable
fun <T> ColumnMajorGrid(
    items: List<T>,
    modifier: Modifier = Modifier,
    numColumns: Int = 3,
    horizontalSpacing: Dp = 12.dp,
    verticalSpacing: Dp = 18.dp,
    content: @Composable (index: Int, item: T) -> Unit,
) {
    require(items.size % numColumns == 0) {
        "Item count (${items.size}) must be divisible by $numColumns"
    }
    val itemsPerColumn = items.size / numColumns

    Row(
        modifier = modifier.fillMaxWidth(),
        horizontalArrangement = Arrangement.spacedBy(horizontalSpacing),
    ) {
        repeat(numColumns) { col ->
            Column(
                modifier = Modifier.weight(1f),
                verticalArrangement = Arrangement.spacedBy(verticalSpacing),
            ) {
                repeat(itemsPerColumn) { row ->
                    val index = col * itemsPerColumn + row
                    if (index < items.size) {
                        content(index, items[index])
                    }
                }
            }
        }
    }
}
