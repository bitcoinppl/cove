package org.bitcoinppl.cove.flows.keyteleport

import org.junit.Assert.assertEquals
import org.junit.Test

class SensitiveClipboardPolicyTest {
    @Test
    fun persistedExpiryRestoresRemainingLifetime() {
        assertEquals(
            RestoredSensitiveClipboardExpiry("token", 30_000),
            restoreSensitiveClipboardExpiry(
                token = "token",
                expiresAtEpochMillis = 130_000,
                nowEpochMillis = 100_000,
            ),
        )
    }

    @Test
    fun expiredPersistedExpiryRestoresForImmediateClear() {
        assertEquals(
            RestoredSensitiveClipboardExpiry("token", 0),
            restoreSensitiveClipboardExpiry(
                token = "token",
                expiresAtEpochMillis = 90_000,
                nowEpochMillis = 100_000,
            ),
        )
    }

    @Test
    fun backwardClockChangeCannotExtendRestoredLifetime() {
        assertEquals(
            RestoredSensitiveClipboardExpiry(
                "token",
                SENSITIVE_CLIPBOARD_LIFETIME_MILLIS,
            ),
            restoreSensitiveClipboardExpiry(
                token = "token",
                expiresAtEpochMillis = 1_000_000,
                nowEpochMillis = 100_000,
            ),
        )
    }

    @Test
    fun incompletePersistedExpiryIsDiscarded() {
        assertEquals(
            null,
            restoreSensitiveClipboardExpiry(
                token = null,
                expiresAtEpochMillis = 130_000,
                nowEpochMillis = 100_000,
            ),
        )
        assertEquals(
            null,
            restoreSensitiveClipboardExpiry(
                token = "token",
                expiresAtEpochMillis = 0,
                nowEpochMillis = 100_000,
            ),
        )
    }

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
