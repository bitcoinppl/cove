package org.bitcoinppl.cove.flows.keyteleport

import org.junit.Assert.assertEquals
import org.junit.Test

class SensitiveClipboardPolicyTest {
    @Test
    fun readableClipOwnedByCoveIsCleared() {
        assertEquals(
            SensitiveClipboardClearDecision.ClearAndForget,
            resolveSensitiveClipboardClear(
                descriptionReadable = true,
                tokenMatches = true,
                remainingLifetimeMillis = 0,
                retriesAfterDeadline = 0,
            ),
        )
    }

    @Test
    fun readableClipOwnedByAnotherAppIsLeftAlone() {
        assertEquals(
            SensitiveClipboardClearDecision.ForgetOnly,
            resolveSensitiveClipboardClear(
                descriptionReadable = true,
                tokenMatches = false,
                remainingLifetimeMillis = 0,
                retriesAfterDeadline = 0,
            ),
        )
    }

    @Test
    fun unreadableClipBeforeDeadlineRetriesAtTheDeadline() {
        assertEquals(
            SensitiveClipboardClearDecision.Retry(30_000),
            resolveSensitiveClipboardClear(
                descriptionReadable = false,
                tokenMatches = false,
                remainingLifetimeMillis = 30_000,
                retriesAfterDeadline = 0,
            ),
        )
    }

    @Test
    fun unreadableClipNearTheDeadlineRetriesNoFasterThanTheRetryInterval() {
        assertEquals(
            SensitiveClipboardClearDecision.Retry(SENSITIVE_CLIPBOARD_RETRY_MILLIS),
            resolveSensitiveClipboardClear(
                descriptionReadable = false,
                tokenMatches = false,
                remainingLifetimeMillis = 200,
                retriesAfterDeadline = 0,
            ),
        )
    }

    @Test
    fun unreadableClipPastTheDeadlineRetriesWhileBudgetRemains() {
        assertEquals(
            SensitiveClipboardClearDecision.Retry(SENSITIVE_CLIPBOARD_RETRY_MILLIS),
            resolveSensitiveClipboardClear(
                descriptionReadable = false,
                tokenMatches = false,
                remainingLifetimeMillis = -5_000,
                retriesAfterDeadline = SENSITIVE_CLIPBOARD_MAX_RETRIES_AFTER_DEADLINE - 1,
            ),
        )
    }

    @Test
    fun unreadableClipDefersToForegroundOnceRetriesAreExhausted() {
        assertEquals(
            SensitiveClipboardClearDecision.KeepForForeground,
            resolveSensitiveClipboardClear(
                descriptionReadable = false,
                tokenMatches = false,
                remainingLifetimeMillis = -5_000,
                retriesAfterDeadline = SENSITIVE_CLIPBOARD_MAX_RETRIES_AFTER_DEADLINE,
            ),
        )
    }
}
