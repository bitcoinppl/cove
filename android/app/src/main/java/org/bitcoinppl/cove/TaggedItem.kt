package org.bitcoinppl.cove

import java.util.UUID

/**
 * wrapper that adds a unique identifier to any item
 * useful for managing state with compose where items need stable identity
 * ported from iOS TaggedItem.swift
 */
data class TaggedItem<T>(
    val id: String = UUID.randomUUID().toString(),
    val item: T,
) {
    constructor(item: T) : this(UUID.randomUUID().toString(), item)
}
