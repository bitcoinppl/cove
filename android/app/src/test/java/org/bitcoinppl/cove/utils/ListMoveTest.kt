package org.bitcoinppl.cove.utils

import org.junit.Assert.assertEquals
import org.junit.Test

class ListMoveTest {
    @Test
    fun movedWithinPrefixOnlyReordersVisibleItems() {
        val reordered = listOf(1, 2, 3, 4, 5, 6, 7)
            .movedWithinPrefix(prefixSize = 5, fromIndex = 1, toIndex = 3)

        assertEquals(listOf(1, 3, 4, 2, 5, 6, 7), reordered)
    }

    @Test
    fun movedWithinPrefixDoesNotMoveOverflowItemsIntoVisibleItems() {
        val reordered = listOf(1, 2, 3, 4, 5, 6, 7)
            .movedWithinPrefix(prefixSize = 5, fromIndex = 6, toIndex = 0)

        assertEquals(listOf(1, 2, 3, 4, 5, 6, 7), reordered)
    }
}
