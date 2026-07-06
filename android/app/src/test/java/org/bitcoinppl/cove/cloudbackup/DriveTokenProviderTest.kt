package org.bitcoinppl.cove.cloudbackup

import java.net.HttpURLConnection
import kotlinx.coroutines.runBlocking
import okhttp3.mockwebserver.RecordedRequest
import org.bitcoinppl.cove_core.device.CloudAccessPolicy
import org.bitcoinppl.cove_core.device.CloudStorageException
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class DriveTokenProviderTest {
    @Test
    fun tokenAcquisitionFailuresPreserveAuthorizationMessages() =
        runBlocking {
            val storage =
                AndroidCloudStorageAccess(
                    FailingDriveAuthorization(AuthorizationRequiredException("consent required")),
                    TestDriveAccountBindingStore(),
                )

            val error = captureError {
                storage.listNamespaces(CloudAccessPolicy.SILENT)
            }

            assertTrue(error is CloudStorageException.AuthorizationRequired)
            assertEquals(
                "consent required",
                (error as CloudStorageException.AuthorizationRequired).v1,
            )
        }

    @Test
    fun unauthorizedDriveAccountIdentityLookupClearsTokenAndRetries() =
        runBlocking {
            assertDriveAccountIdentityLookupClearsTokenAndRetries(
                statusCode = HttpURLConnection.HTTP_UNAUTHORIZED,
                body = "expired token",
            )
        }

    @Test
    fun rejectedDriveAccountIdentityLookupClearsTokenAndRetries() =
        runBlocking {
            assertDriveAccountIdentityLookupClearsTokenAndRetries(
                statusCode = HttpURLConnection.HTTP_FORBIDDEN,
                body = driveErrorBody("insufficientPermissions"),
            )
        }

    @Test
    fun repeatedRejectedDriveAccountIdentityLookupIdentifiesVerificationFailure() =
        runBlocking {
            MockDriveServer().use { server ->
                server.enqueue(HttpURLConnection.HTTP_FORBIDDEN, driveErrorBody("insufficientPermissions"))
                server.enqueue(HttpURLConnection.HTTP_FORBIDDEN, driveErrorBody("insufficientPermissions"))

                val authorization = SequentialDriveAuthorization(listOf("token-1", "token-2"))
                val storage = storageFor(server, authorization)

                val error = captureError {
                    storage.listNamespaces(CloudAccessPolicy.SILENT)
                }

                assertTrue(error is CloudStorageException.AuthorizationRequired)
                assertEquals(
                    "google drive identity verification was rejected",
                    (error as CloudStorageException.AuthorizationRequired).v1,
                )
                assertEquals(listOf(false, false), authorization.accessRequests)
                assertEquals(listOf("token-1", "token-2"), authorization.clearedTokens)
                assertEquals(listOf("/about", "/about"), requestPaths(server.requests(2)))
            }
        }

    @Test
    fun resolvedDriveAccountIdentityIsCachedAcrossOperations() =
        runBlocking {
            MockDriveServer().use { server ->
                enqueueIdentity(server)
                server.enqueue(HttpURLConnection.HTTP_OK, """{"files":[]}""")
                server.enqueue(HttpURLConnection.HTTP_OK, """{"files":[]}""")

                val delegate = RecordingDriveAuthorization().apply {
                    account = null
                }
                val authorization = CachingDriveAuthorization(
                    delegate = delegate,
                    elapsedRealtime = { 0 },
                    cacheWindowMs = 1_000,
                )
                val storage = storageFor(server, authorization)

                assertEquals(emptyList<String>(), storage.listNamespaces(CloudAccessPolicy.SILENT))
                assertEquals(emptyList<String>(), storage.listNamespaces(CloudAccessPolicy.SILENT))
                assertEquals(listOf(false), delegate.accessRequests)

                val requests = server.requests(3)
                assertEquals(listOf("/about", "/files", "/files"), requestPaths(requests))
                assertEquals(List(3) { "Bearer token-1" }, authorizations(requests))
            }
        }

    @Test
    fun sparseDriveAccountIdentityIsResolvedBeforeVerification() =
        runBlocking {
            MockDriveServer().use { server ->
                enqueueIdentity(server)
                server.enqueue(HttpURLConnection.HTTP_OK, """{"files":[]}""")
                server.enqueue(HttpURLConnection.HTTP_OK, """{"files":[]}""")

                val store = TestDriveAccountBindingStore(
                    DriveAccountIdentity(googleAccountId = null, email = "person@example.com"),
                )
                val delegate = RecordingDriveAuthorization().apply {
                    account = DriveAccountIdentity(googleAccountId = "account-1", email = null)
                }
                val authorization = cachingAuthorization(delegate, store)
                val storage = storageFor(server, authorization, store)

                val firstResult = runCatching { storage.listNamespaces(CloudAccessPolicy.SILENT) }
                val secondResult = runCatching { storage.listNamespaces(CloudAccessPolicy.SILENT) }

                assertSuccessfulEmptyList(firstResult)
                assertSuccessfulEmptyList(secondResult)
                assertEquals(listOf(false), delegate.accessRequests)
                assertTrue(delegate.clearedTokens.isEmpty())

                val requests = server.requests(3)
                assertEquals(listOf("/about", "/files", "/files"), requestPaths(requests))
                assertEquals(List(3) { "Bearer token-1" }, authorizations(requests))
            }
        }

    @Test
    fun storedDrivePermissionIdIsResolvedBeforeBindingVerification() =
        runBlocking {
            MockDriveServer().use { server ->
                enqueuePermissionOnlyIdentity(server)
                server.enqueue(HttpURLConnection.HTTP_OK, """{"files":[]}""")

                val store = TestDriveAccountBindingStore(
                    DriveAccountIdentity(drivePermissionId = "permission-1", email = null),
                )
                val delegate = RecordingDriveAuthorization().apply {
                    account = DriveAccountIdentity(googleAccountId = "account-1", email = "person@example.com")
                }
                val storage = storageFor(server, cachingAuthorization(delegate, store), store)

                val result = runCatching { storage.listNamespaces(CloudAccessPolicy.SILENT) }

                assertSuccessfulEmptyList(result)
                assertEquals(listOf(false), delegate.accessRequests)
                assertTrue(delegate.clearedTokens.isEmpty())

                val requests = server.requests(2)
                assertEquals(listOf("/about", "/files"), requestPaths(requests))
                assertTrue(requests.first().path.orEmpty().contains("permissionId"))
                assertEquals(List(2) { "Bearer token-1" }, authorizations(requests))
            }
        }

    @Test
    fun constrainedAuthorizationWithoutIdentityIsVerifiedThroughDriveAbout() =
        runBlocking {
            MockDriveServer().use { server ->
                enqueueOtherIdentity(server)

                val selected = DriveAccountIdentity(googleAccountId = null, email = "person@example.com")
                val authorization = RecordingDriveAuthorization().apply {
                    account = null
                }
                val storage = storageFor(server, authorization, TestDriveAccountBindingStore(selected))

                val error = captureError {
                    storage.listNamespaces(CloudAccessPolicy.SILENT)
                }

                assertTrue(error is CloudStorageException.AuthorizationRequired)
                assertEquals(
                    "google drive account does not match the account selected for Cloud Backup",
                    (error as CloudStorageException.AuthorizationRequired).v1,
                )
                assertEquals(listOf("token-1"), authorization.clearedTokens)

                val requests = server.requests(1)
                assertEquals(listOf("/about"), requestPaths(requests))
                assertTrue(requests.single().path.orEmpty().contains("permissionId"))
            }
        }

    @Test
    fun unboundReadOnlyOperationsReuseResolvedDriveAccountIdentity() =
        runBlocking {
            MockDriveServer().use { server ->
                enqueueIdentity(server)
                server.enqueue(HttpURLConnection.HTTP_OK, """{"files":[]}""")
                server.enqueue(HttpURLConnection.HTTP_OK, """{"files":[]}""")

                val store = TestDriveAccountBindingStore()
                val delegate = RecordingDriveAuthorization().apply {
                    account = null
                }
                val authorization = CachingDriveAuthorization(
                    delegate = delegate,
                    elapsedRealtime = { 0 },
                    cacheWindowMs = 1_000,
                    cacheKey = { driveAccountTokenCacheKey(store) },
                )
                val storage = storageFor(server, authorization, store)

                assertEquals(emptyList<String>(), storage.listNamespaces(CloudAccessPolicy.SILENT))
                assertEquals(emptyList<String>(), storage.listNamespaces(CloudAccessPolicy.SILENT))
                assertEquals(null, store.selectedIdentity())
                assertEquals(listOf(false), delegate.accessRequests)

                val requests = server.requests(3)
                assertEquals(listOf("/about", "/files", "/files"), requestPaths(requests))
                assertEquals(List(3) { "Bearer token-1" }, authorizations(requests))
            }
        }

    @Test
    fun cachedWrongDriveAccountTokenIsClearedAfterMismatch() =
        runBlocking {
            val selected = DriveAccountIdentity(googleAccountId = "account-1", email = "person@example.com")
            val actual = DriveAccountIdentity(googleAccountId = "account-2", email = "other@example.com")
            val store = TestDriveAccountBindingStore(selected)
            val delegate = RecordingDriveAuthorization().apply {
                account = actual
            }
            val storage = AndroidCloudStorageAccess(cachingAuthorization(delegate, store), store)

            repeat(2) {
                val error = captureError {
                    storage.listNamespaces(CloudAccessPolicy.SILENT)
                }

                assertTrue(error is CloudStorageException.AuthorizationRequired)
            }

            assertEquals(listOf(false, false), delegate.accessRequests)
            assertEquals(listOf("token-1", "token-1"), delegate.clearedTokens)
        }

    private suspend fun assertDriveAccountIdentityLookupClearsTokenAndRetries(
        statusCode: Int,
        body: String,
    ) {
        MockDriveServer().use { server ->
            server.enqueue(statusCode, body)
            enqueueEmailOnlyIdentity(server)
            server.enqueue(HttpURLConnection.HTTP_OK, """{"files":[]}""")

            val authorization = SequentialDriveAuthorization(listOf("token-1", "token-2"))
            val storage = storageFor(server, authorization)

            val namespacesResult = runCatching {
                storage.listNamespaces(CloudAccessPolicy.SILENT)
            }

            assertSuccessfulEmptyList(namespacesResult)
            assertEquals(listOf(false, false), authorization.accessRequests)
            assertEquals(listOf("token-1"), authorization.clearedTokens)

            val requests = server.requests(3)
            assertEquals(listOf("/about", "/about", "/files"), requestPaths(requests))
            assertEquals(
                listOf("Bearer token-1", "Bearer token-2", "Bearer token-2"),
                authorizations(requests),
            )
        }
    }

    private fun storageFor(
        server: MockDriveServer,
        authorization: DriveAuthorization,
        store: TestDriveAccountBindingStore = TestDriveAccountBindingStore(),
    ): AndroidCloudStorageAccess =
        AndroidCloudStorageAccess(
            driveAuthorization = authorization,
            accountBindingStore = store,
            driveApiEndpoints = DriveApiEndpoints(
                aboutEndpoint = "${server.baseUrl}/about",
                filesEndpoint = "${server.baseUrl}/files",
                uploadEndpoint = "${server.baseUrl}/upload",
            ),
            drivePathNamesProvider = { testDrivePathNames },
        )

    private fun cachingAuthorization(
        delegate: RecordingDriveAuthorization,
        store: TestDriveAccountBindingStore,
    ): CachingDriveAuthorization =
        CachingDriveAuthorization(
            delegate = delegate,
            elapsedRealtime = { 0 },
            cacheWindowMs = 1_000,
            cacheKey = { store.selectedIdentity() },
        )

    private fun assertSuccessfulEmptyList(result: Result<List<String>>) {
        assertTrue(
            cloudStorageFailureMessage(result.exceptionOrNull()),
            result.isSuccess,
        )
        assertEquals(emptyList<String>(), result.getOrNull())
    }

    private fun requestPaths(requests: List<RecordedRequest>): List<String> =
        requests.map { it.path.orEmpty().substringBefore("?") }

    private fun authorizations(requests: List<RecordedRequest>): List<String?> =
        requests.map { it.getHeader("Authorization") }

    private fun enqueueIdentity(server: MockDriveServer) {
        server.enqueue(
            HttpURLConnection.HTTP_OK,
            """
            {
                "user": {
                    "emailAddress": "person@example.com",
                    "permissionId": "account-1"
                }
            }
            """.trimIndent(),
        )
    }

    private fun enqueueEmailOnlyIdentity(server: MockDriveServer) {
        server.enqueue(
            HttpURLConnection.HTTP_OK,
            """
            {
                "user": {
                    "emailAddress": "person@example.com"
                }
            }
            """.trimIndent(),
        )
    }

    private fun enqueuePermissionOnlyIdentity(server: MockDriveServer) {
        server.enqueue(
            HttpURLConnection.HTTP_OK,
            """
            {
                "user": {
                    "permissionId": "permission-1"
                }
            }
            """.trimIndent(),
        )
    }

    private fun enqueueOtherIdentity(server: MockDriveServer) {
        server.enqueue(
            HttpURLConnection.HTTP_OK,
            """
            {
                "user": {
                    "emailAddress": "other@example.com",
                    "permissionId": "account-2"
                }
            }
            """.trimIndent(),
        )
    }
}
