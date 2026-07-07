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
    if (itemCount <= 1 || fromIndex !in 0 until itemCount) {
        return this
    }

    val boundedToIndex = toIndex.coerceIn(0, itemCount - 1)
    if (fromIndex == boundedToIndex) {
        return this
    }

    return take(itemCount).moved(fromIndex, boundedToIndex) + drop(itemCount)
}
