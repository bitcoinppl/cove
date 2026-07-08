package org.bitcoinppl.cove.utils

internal fun <T> List<T>.moved(
    fromIndex: Int,
    toIndex: Int,
): List<T> =
    toMutableList().apply {
        add(toIndex, removeAt(fromIndex))
    }

internal fun <T> List<T>.movedWithinPrefix(
    prefixSize: Int,
    fromIndex: Int,
    toIndex: Int,
): List<T> {
    val itemCount = minOf(size, prefixSize)
    val canMove = itemCount > 1 && fromIndex in 0 until itemCount
    val boundedToIndex = if (canMove) toIndex.coerceIn(0, itemCount - 1) else fromIndex

    return if (canMove && fromIndex != boundedToIndex) {
        take(itemCount).moved(fromIndex, boundedToIndex) + drop(itemCount)
    } else {
        this
    }
}
