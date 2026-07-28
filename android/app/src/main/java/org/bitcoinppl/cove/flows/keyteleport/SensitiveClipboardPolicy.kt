package org.bitcoinppl.cove.flows.keyteleport

internal const val SENSITIVE_CLIPBOARD_LIFETIME_MILLIS = 60_000L
internal const val SENSITIVE_CLIPBOARD_RETRY_MILLIS = 2_000L
internal const val SENSITIVE_CLIPBOARD_MAX_RETRIES_AFTER_DEADLINE = 5

internal data class RestoredSensitiveClipboardExpiry(
    val token: String,
    val remainingLifetimeMillis: Long,
)

internal fun restoreSensitiveClipboardExpiry(
    token: String?,
    expiresAtEpochMillis: Long,
    nowEpochMillis: Long,
): RestoredSensitiveClipboardExpiry? {
    if (token.isNullOrBlank() || expiresAtEpochMillis <= 0) return null

    val remainingLifetimeMillis =
        (expiresAtEpochMillis - nowEpochMillis).coerceIn(
            minimumValue = 0,
            maximumValue = SENSITIVE_CLIPBOARD_LIFETIME_MILLIS,
        )

    return RestoredSensitiveClipboardExpiry(token, remainingLifetimeMillis)
}

internal sealed interface SensitiveClipboardClearDecision {
    data object ClearAndForget : SensitiveClipboardClearDecision

    data object ForgetOnly : SensitiveClipboardClearDecision

    data class Retry(
        val delayMillis: Long,
    ) : SensitiveClipboardClearDecision

    data object KeepForForeground : SensitiveClipboardClearDecision
}

/**
 * Decide what to do with a clip Cove marked sensitive.
 *
 * An unreadable clip description is inconclusive rather than proof that another app took over the
 * clipboard: Android 10+ denies background reads, so the token is kept and the clear is retried a
 * bounded number of times before deferring to the next foreground sweep.
 */
internal fun resolveSensitiveClipboardClear(
    descriptionReadable: Boolean,
    tokenMatches: Boolean,
    remainingLifetimeMillis: Long,
    retriesAfterDeadline: Int,
): SensitiveClipboardClearDecision =
    when {
        descriptionReadable && tokenMatches -> SensitiveClipboardClearDecision.ClearAndForget
        descriptionReadable -> SensitiveClipboardClearDecision.ForgetOnly

        remainingLifetimeMillis > 0 ->
            SensitiveClipboardClearDecision.Retry(
                maxOf(remainingLifetimeMillis, SENSITIVE_CLIPBOARD_RETRY_MILLIS),
            )

        retriesAfterDeadline >= SENSITIVE_CLIPBOARD_MAX_RETRIES_AFTER_DEADLINE ->
            SensitiveClipboardClearDecision.KeepForForeground

        else -> SensitiveClipboardClearDecision.Retry(SENSITIVE_CLIPBOARD_RETRY_MILLIS)
    }
