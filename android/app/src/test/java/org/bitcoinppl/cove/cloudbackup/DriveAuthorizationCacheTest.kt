package org.bitcoinppl.cove.cloudbackup

import kotlinx.coroutines.async
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.test.runTest
import kotlinx.coroutines.yield
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Test

class DriveAuthorizationCacheTest {
    @Test
    fun cachingDriveAuthorizationReusesUpdatedTokenIdentity() =
        runBlocking {
            var now = 0L
            val delegate = RecordingDriveAuthorization().apply {
                account = null
            }
            val authorization = CachingDriveAuthorization(
                delegate = delegate,
                elapsedRealtime = { now },
                cacheWindowMs = 1_000,
            )
            val unresolved = authorization.accessToken(interactive = false)
            val resolved = unresolved.copy(
                account = DriveAccountIdentity(googleAccountId = null, email = "person@example.com"),
            )

            authorization.updateCachedToken(resolved)

            now = 500
            assertEquals(resolved, authorization.accessToken(interactive = false))
            assertEquals(listOf(false), delegate.accessRequests)
        }

    @Test
    fun cachingDriveAuthorizationDropsUpdatedTokenWhenCacheKeyChanges() =
        runBlocking {
            var cacheKey: Any? = "account-1"
            val delegate = RecordingDriveAuthorization().apply {
                account = null
            }
            val authorization = CachingDriveAuthorization(
                delegate = delegate,
                elapsedRealtime = { 0 },
                cacheWindowMs = 1_000,
                cacheKey = { cacheKey },
            )
            val unresolved = authorization.accessToken(interactive = false)
            val resolved = unresolved.copy(
                account = DriveAccountIdentity(googleAccountId = null, email = "person@example.com"),
            )

            cacheKey = "account-2"
            authorization.updateCachedToken(resolved)
            delegate.token = "token-2"

            assertEquals("token-2", authorization.accessToken(interactive = false).token)
            assertEquals(listOf(false, false), delegate.accessRequests)
        }

    @Test
    fun cachingDriveAuthorizationDropsUpdatedTokenWhenTokenChanges() =
        runBlocking {
            val delegate = RecordingDriveAuthorization().apply {
                account = null
            }
            val authorization = CachingDriveAuthorization(
                delegate = delegate,
                elapsedRealtime = { 0 },
                cacheWindowMs = 1_000,
            )
            val unresolved = authorization.accessToken(interactive = false)
            val resolved = unresolved.copy(
                token = "token-2",
                account = DriveAccountIdentity(googleAccountId = null, email = "person@example.com"),
            )

            authorization.updateCachedToken(resolved)
            delegate.token = "token-3"

            assertEquals("token-3", authorization.accessToken(interactive = false).token)
            assertEquals(listOf(false, false), delegate.accessRequests)
        }

    @Test
    fun cachingDriveAuthorizationDropsUpdatedTokenAfterExpiry() =
        runBlocking {
            var now = 0L
            val delegate = RecordingDriveAuthorization().apply {
                account = null
            }
            val authorization = CachingDriveAuthorization(
                delegate = delegate,
                elapsedRealtime = { now },
                cacheWindowMs = 1_000,
            )
            val unresolved = authorization.accessToken(interactive = false)
            val resolved = unresolved.copy(
                account = DriveAccountIdentity(googleAccountId = null, email = "person@example.com"),
            )

            now = 1_000
            authorization.updateCachedToken(resolved)
            delegate.token = "token-2"

            assertEquals("token-2", authorization.accessToken(interactive = false).token)
            assertEquals(listOf(false, false), delegate.accessRequests)
        }

    @Test
    fun cachingDriveAuthorizationDropsUpdatedTokenWhenCacheKeyIsUnavailable() =
        runBlocking {
            var cacheKey: Any? = "account-1"
            val delegate = RecordingDriveAuthorization().apply {
                account = null
            }
            val authorization = CachingDriveAuthorization(
                delegate = delegate,
                elapsedRealtime = { 0 },
                cacheWindowMs = 1_000,
                cacheKey = { cacheKey },
            )
            val unresolved = authorization.accessToken(interactive = false)
            val resolved = unresolved.copy(
                account = DriveAccountIdentity(googleAccountId = null, email = "person@example.com"),
            )

            cacheKey = null
            authorization.updateCachedToken(resolved)
            delegate.token = "token-2"

            assertEquals("token-2", authorization.accessToken(interactive = false).token)
            assertEquals(listOf(false, false), delegate.accessRequests)
        }

    @Test
    fun cachingDriveAuthorizationReusesTokenUntilCleared() =
        runBlocking {
            var now = 0L
            val delegate = RecordingDriveAuthorization()
            val authorization = CachingDriveAuthorization(
                delegate = delegate,
                elapsedRealtime = { now },
                cacheWindowMs = 1_000,
            )

            assertEquals("token-1", authorization.accessToken(interactive = false).token)
            assertEquals("token-1", authorization.accessToken(interactive = true).token)
            assertEquals(listOf(false), delegate.accessRequests)

            now = 500
            delegate.token = "token-2"
            authorization.clearToken("other-token")
            assertEquals("token-1", authorization.accessToken(interactive = false).token)
            assertEquals(listOf(false), delegate.accessRequests)

            authorization.clearToken("token-1")
            assertEquals("token-2", authorization.accessToken(interactive = false).token)
            assertEquals(listOf(false, false), delegate.accessRequests)
            assertEquals(listOf("other-token", "token-1"), delegate.clearedTokens)
        }

    @Test
    fun cachingDriveAuthorizationExpiresIdleToken() =
        runBlocking {
            var now = 0L
            val delegate = RecordingDriveAuthorization()
            val authorization = CachingDriveAuthorization(
                delegate = delegate,
                elapsedRealtime = { now },
                cacheWindowMs = 1_000,
            )

            assertEquals("token-1", authorization.accessToken(interactive = false).token)

            now = 999
            assertEquals("token-1", authorization.accessToken(interactive = false).token)

            now = 2_000
            delegate.token = "token-2"
            assertEquals("token-2", authorization.accessToken(interactive = false).token)
            assertEquals(listOf(false, false), delegate.accessRequests)
        }

    @Test
    fun cachingDriveAuthorizationDoesNotCacheWhenCacheKeyIsUnavailable() =
        runBlocking {
            val delegate = RecordingDriveAuthorization()
            val authorization = CachingDriveAuthorization(
                delegate = delegate,
                elapsedRealtime = { 0 },
                cacheWindowMs = 1_000,
                cacheKey = { null },
            )

            assertEquals("token-1", authorization.accessToken(interactive = false).token)

            delegate.token = "token-2"

            assertEquals("token-2", authorization.accessToken(interactive = false).token)
            assertEquals(listOf(false, false), delegate.accessRequests)
        }

    @Test
    fun cachingDriveAuthorizationDoesNotRefreshWhileClearIsRunning() =
        runTest {
            val delegate = BlockingClearDriveAuthorization()
            val authorization = CachingDriveAuthorization(
                delegate = delegate,
                elapsedRealtime = { 0 },
                cacheWindowMs = 1_000,
            )

            assertEquals("token-1", authorization.accessToken(interactive = false).token)

            delegate.token = "token-2"
            val clear = async { authorization.clearToken("token-1") }
            delegate.clearStarted.await()

            val refresh = async { authorization.accessToken(interactive = false).token }
            yield()

            assertFalse(refresh.isCompleted)

            delegate.finishClear.complete(Unit)
            clear.await()

            assertEquals("token-2", refresh.await())
            assertEquals(listOf(false, false), delegate.accessRequests)
            assertEquals(listOf("token-1"), delegate.clearedTokens)
        }

    @Test
    fun clearingDriveAccountBindingInvalidatesCachedToken() =
        runBlocking {
            val first = DriveAccountIdentity(googleAccountId = "account-1", email = "person@example.com")
            val second = DriveAccountIdentity(googleAccountId = "account-2", email = "other@example.com")
            val store = TestDriveAccountBindingStore(first)
            val delegate = RecordingDriveAuthorization()
            val authorization = CachingDriveAuthorization(
                delegate = delegate,
                elapsedRealtime = { 0 },
                cacheWindowMs = 1_000,
                cacheKey = { store.selectedIdentity() },
            )

            assertEquals("token-1", authorization.accessToken(interactive = false).token)
            assertEquals("token-1", authorization.accessToken(interactive = false).token)

            store.clearIdentity()
            delegate.token = "token-2"
            delegate.account = second

            assertEquals("token-2", authorization.accessToken(interactive = false).token)
            delegate.token = "token-3"
            assertEquals("token-3", authorization.accessToken(interactive = false).token)
            assertEquals(listOf(false, false, false), delegate.accessRequests)
        }
}
