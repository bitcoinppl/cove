package org.bitcoinppl.cove

import java.util.UUID

/**
 * wrapper that adds a unique ID to any item
 * useful for tracking alerts/sheets that need to be shown multiple times
 * similar to Swift's Identifiable protocol
 */
data class TaggedItem<T>(
    val id: String = UUID.randomUUID().toString(),
    val item: T
) {
    constructor(item: T) : this(UUID.randomUUID().toString(), item)
}

typealias TaggedString = TaggedItem<String>

val TaggedString.value: String
    get() = item
