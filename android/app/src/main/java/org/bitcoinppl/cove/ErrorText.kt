package org.bitcoinppl.cove

import org.bitcoinppl.cove_core.uniffiErrorText

/**
 * Text describing this error for people to read
 *
 * A structured UniFFI error's `message` is a field dump such as `v1=...`, so UniFFI errors use
 * their Rust `Display` text, and other throwables use their message or else `fallback`
 */
internal fun Throwable.displayMessage(fallback: String = javaClass.simpleName): String =
    uniffiErrorText() ?: localizedMessage ?: fallback
