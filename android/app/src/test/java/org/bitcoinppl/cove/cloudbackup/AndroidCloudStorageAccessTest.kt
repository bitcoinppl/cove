package org.bitcoinppl.cove.cloudbackup

import com.google.android.gms.common.api.ApiException
import com.google.android.gms.common.api.CommonStatusCodes
import com.google.android.gms.common.api.Status
import java.io.BufferedReader
import java.io.IOException
import java.io.InputStreamReader
import java.net.HttpURLConnection
import java.net.ServerSocket
import java.net.SocketException
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.async
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.test.runTest
import kotlinx.coroutines.yield
import org.bitcoinppl.cove_core.device.CloudAccessPolicy
import org.bitcoinppl.cove_core.device.CloudStorageException
import org.bitcoinppl.cove_core.device.RemoteBackupLocation
import org.json.JSONObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class AndroidCloudStorageAccessTest {
    private val testDrivePathNames = DrivePathNames(
        namespacesRootFolderName = "cspp-namespaces",
        masterKeyFolderName = "master-key",
        walletsFolderName = "wallets",
        walletFilePrefix = "wallet-",
    )

    @Test
    fun createUploadMetadataIncludesParents() {
        val metadata = createUploadMetadata(fileName = "wallet-record.json", parentId = "folder-123")

        assertEquals("wallet-record.json", metadata.name)
        assertEquals(listOf("folder-123"), metadata.parents)

        val json = metadata.toJson()
        assertEquals("wallet-record.json", json.getString("name"))
        assertEquals("folder-123", json.getJSONArray("parents").getString(0))
    }

    @Test
    fun overwriteUploadMetadataOmitsParents() {
        val metadata = overwriteUploadMetadata(fileName = "wallet-record.json")

        assertEquals("wallet-record.json", metadata.name)
        assertEquals(emptyList<String>(), metadata.parents)

        val json = metadata.toJson()
        assertEquals("wallet-record.json", json.getString("name"))
        assertFalse(json.has("parents"))
    }

    @Test
    fun driveAboutResponseProvidesAccountEmail() {
        val response =
            JSONObject(
                """
                {
                    "user": {
                        "emailAddress": "Person@Example.com"
                    }
                }
                """.trimIndent(),
            )

        assertEquals(
            DriveAccountIdentity(id = null, email = "Person@Example.com"),
            driveAccountIdentityFromAboutResponse(response),
        )
    }

    @Test
    fun driveLocationPartsKeepsFlatFilesAtNamespaceRoot() {
        assertEquals(
            DriveLocationParts(parentFolders = emptyList(), fileName = "wallet-record.json"),
            driveLocationParts("wallet-record.json"),
        )
    }

    @Test
    fun driveLocationPartsSplitsKindPrefixedFiles() {
        assertEquals(
            DriveLocationParts(parentFolders = listOf("wallets"), fileName = "wallet-record.json"),
            driveLocationParts("wallets/wallet-record.json"),
        )
    }

    @Test
    fun driveLocationPartsRejectsParentTraversal() {
        val error = runCatching { driveLocationParts("wallets/../wallet-record.json") }
            .exceptionOrNull()

        assertTrue(error is IllegalArgumentException)
    }

    @Test
    fun driveLocationPartsRejectsBlankRelativePath() {
        val error = runCatching { driveLocationParts("") }
            .exceptionOrNull()

        assertTrue(error is IllegalArgumentException)
        assertEquals("relativePath must not be blank", error?.message)
    }

    @Test
    fun drivePathsAcceptLegacyFlatAndKindPrefixedWalletLocations() {
        assertTrue(
            isWalletFileLocation(
                location = "wallet-record.json",
                walletFilePrefix = "wallet-",
                walletsFolderName = "wallets",
            ),
        )
        assertTrue(
            isWalletFileLocation(
                location = "wallets/wallet-record.json",
                walletFilePrefix = "wallet-",
                walletsFolderName = "wallets",
            ),
        )
        assertFalse(
            isWalletFileLocation(
                location = "master-key/wallet-record.json",
                walletFilePrefix = "wallet-",
                walletsFolderName = "wallets",
            ),
        )
    }

    @Test
    fun tokenAcquisitionFailuresPreserveAuthorizationMessages() =
        runBlocking {
            val storage =
                AndroidCloudStorageAccess(
                    FailingDriveAuthorization(AuthorizationRequiredException("consent required")),
                    TestDriveAccountBindingStore(),
                )

            val error =
                try {
                    storage.listNamespaces(CloudAccessPolicy.SILENT)
                    null
                } catch (error: Throwable) {
                    error
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
                account = DriveAccountIdentity(id = null, email = "person@example.com"),
            )

            authorization.updateCachedToken(resolved)

            now = 500
            assertEquals(resolved, authorization.accessToken(interactive = false))
            assertEquals(listOf(false), delegate.accessRequests)
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
    fun authorizationRequiredErrorsPreserveMessagesAcrossOperations() {
        val authorizationError = AuthorizationRequiredException("google drive authorization was cancelled")
        val uploadError = mapDriveUploadError(authorizationError, "wallet-record")
        val listError = mapDriveListError(authorizationError)

        assertTrue(uploadError is CloudStorageException.AuthorizationRequired)
        assertTrue(listError is CloudStorageException.AuthorizationRequired)
        assertEquals(
            "google drive authorization was cancelled",
            (uploadError as CloudStorageException.AuthorizationRequired).v1,
        )
        assertEquals(
            "google drive authorization was cancelled",
            (listError as CloudStorageException.AuthorizationRequired).v1,
        )
    }

    @Test
    fun driveAccountBindingPersistsFirstAccountAndRejectsMismatch() {
        val store = TestDriveAccountBindingStore()
        val first = DriveAccountIdentity(id = "account-1", email = "person@example.com")
        val sameEmailFallback = DriveAccountIdentity(id = null, email = "PERSON@example.com")
        val mismatch = DriveAccountIdentity(id = "account-2", email = "other@example.com")

        verifyDriveAccountBinding(store, first)
        verifyDriveAccountBinding(store, sameEmailFallback)

        val error = runCatching { verifyDriveAccountBinding(store, mismatch) }.exceptionOrNull()

        assertTrue(error is DriveAccountBindingException.Mismatch)
    }

    @Test
    fun driveAccountBindingValidationDoesNotPersistUnverifiedAccount() {
        val store = TestDriveAccountBindingStore()
        val probe = DriveAccountIdentity(id = "account-1", email = "person@example.com")

        verifyDriveAccountBinding(store, probe, bindIfMissing = false)

        assertEquals(null, store.selectedIdentity())
    }

    @Test
    fun driveAccountBindingCanBeClearedAndRebound() {
        val store = TestDriveAccountBindingStore()
        val first = DriveAccountIdentity(id = "account-1", email = "person@example.com")
        val second = DriveAccountIdentity(id = "account-2", email = "other@example.com")

        verifyDriveAccountBinding(store, first)
        store.clearIdentity()
        verifyDriveAccountBinding(store, second)

        assertEquals(second, store.selectedIdentity())
    }

    @Test
    fun driveAccountBindingRejectsMissingIdentityWhenNoAccountIsSelected() {
        val store = TestDriveAccountBindingStore()

        val error = runCatching { verifyDriveAccountBinding(store, identity = null) }
            .exceptionOrNull()

        assertTrue(error is DriveAccountBindingException.MissingIdentity)
    }

    @Test
    fun driveAccountBindingRejectsMissingIdentityWhenAccountIsSelected() {
        val store = TestDriveAccountBindingStore(
            DriveAccountIdentity(id = "account-1", email = "person@example.com"),
        )

        val error = runCatching { verifyDriveAccountBinding(store, identity = null) }
            .exceptionOrNull()

        assertTrue(error is DriveAccountBindingException.MissingIdentity)
    }

    @Test
    fun driveAccountBindingRejectsMissingIdentityWhenSelectedAccountCannotConstrainAuthorization() {
        val store = TestDriveAccountBindingStore(
            DriveAccountIdentity(id = "account-1", email = null),
        )

        val error = runCatching { verifyDriveAccountBinding(store, identity = null) }
            .exceptionOrNull()

        assertTrue(error is DriveAccountBindingException.MissingIdentity)
    }

    @Test
    fun driveAccountBindingEnrichesSparseSelectedAccountAfterMatchingVerification() {
        val store = TestDriveAccountBindingStore(
            DriveAccountIdentity(id = "account-1", email = null),
        )
        val verified = DriveAccountIdentity(id = "account-1", email = "person@example.com")

        verifyDriveAccountBinding(store, verified)

        assertEquals(verified, store.selectedIdentity())
    }

    @Test
    fun driveAccountIdentityUsesConstrainedAccountWhenAuthorizationOmitsIdentity() {
        val selected = DriveAccountIdentity(id = "account-1", email = "person@example.com")

        assertEquals(
            selected,
            resolveDriveAccountIdentity(
                authorizationIdentity = null,
                constrainedIdentity = selected,
            ),
        )
    }

    @Test
    fun driveAccountIdentityRejectsFallbackWhenTokenWasNotConstrained() {
        val selected = DriveAccountIdentity(id = "account-1", email = null)

        assertEquals(
            null,
            resolveDriveAccountIdentity(
                authorizationIdentity = null,
                constrainedIdentity = selected,
            ),
        )
    }

    @Test
    fun driveAccountIdentityPrefersAuthorizationIdentity() {
        val selected = DriveAccountIdentity(id = "account-1", email = "person@example.com")
        val actual = DriveAccountIdentity(id = "account-2", email = "other@example.com")

        assertEquals(
            actual,
            resolveDriveAccountIdentity(
                authorizationIdentity = actual,
                constrainedIdentity = selected,
            ),
        )
    }

    @Test
    fun wrongDriveAccountBlocksOperationsBeforeRemoteMutation() =
        runBlocking {
            val selected = DriveAccountIdentity(id = "account-1", email = "person@example.com")
            val actual = DriveAccountIdentity(id = "account-2", email = "other@example.com")
            val authorization = RecordingDriveAuthorization().apply {
                account = actual
            }
            val storage = AndroidCloudStorageAccess(
                authorization,
                TestDriveAccountBindingStore(selected),
            )
            val namespace = "0123456789abcdef0123456789abcdef"
            val location = RemoteBackupLocation(relativePath = "master-key/master-key.json")

            val operations =
                listOf<suspend () -> Unit>(
                    { storage.listNamespaces(CloudAccessPolicy.SILENT) },
                    {
                        storage.uploadMasterKeyBackup(
                            namespace = namespace,
                            location = location,
                            data = byteArrayOf(1, 2, 3),
                            policy = CloudAccessPolicy.SILENT,
                        )
                    },
                    {
                        storage.downloadMasterKeyBackup(
                            namespace = namespace,
                            locations = listOf(location),
                            policy = CloudAccessPolicy.SILENT,
                        )
                    },
                    {
                        storage.deleteNamespace(
                            namespace = namespace,
                            policy = CloudAccessPolicy.SILENT,
                        )
                    },
                )

            operations.forEach { operation ->
                val error = captureError(operation)

                assertTrue(error is CloudStorageException.AuthorizationRequired)
                assertEquals(
                    "google drive account does not match the account selected for Cloud Backup",
                    (error as CloudStorageException.AuthorizationRequired).v1,
                )
            }
            assertEquals(4, authorization.accessRequests.size)
            assertEquals(List(4) { "token-1" }, authorization.clearedTokens)
        }

    @Test
    fun cachedWrongDriveAccountTokenIsClearedAfterMismatch() =
        runBlocking {
            val selected = DriveAccountIdentity(id = "account-1", email = "person@example.com")
            val actual = DriveAccountIdentity(id = "account-2", email = "other@example.com")
            val store = TestDriveAccountBindingStore(selected)
            val delegate = RecordingDriveAuthorization().apply {
                account = actual
            }
            val authorization = CachingDriveAuthorization(
                delegate = delegate,
                elapsedRealtime = { 0 },
                cacheWindowMs = 1_000,
                cacheKey = { store.selectedIdentity() },
            )
            val storage = AndroidCloudStorageAccess(authorization, store)

            repeat(2) {
                val error = captureError {
                    storage.listNamespaces(CloudAccessPolicy.SILENT)
                }

                assertTrue(error is CloudStorageException.AuthorizationRequired)
            }

            assertEquals(listOf(false, false), delegate.accessRequests)
            assertEquals(listOf("token-1", "token-1"), delegate.clearedTokens)
        }

    @Test
    fun clearingDriveAccountBindingInvalidatesCachedToken() =
        runBlocking {
            val first = DriveAccountIdentity(id = "account-1", email = "person@example.com")
            val second = DriveAccountIdentity(id = "account-2", email = "other@example.com")
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

    @Test
    fun duplicateDriveFolderNamesAreDetected() {
        assertEquals(
            setOf("wallets"),
            duplicateDriveFolderNames(listOf("master-key", "wallets", "wallets")),
        )
        assertTrue(duplicateDriveFolderNames(listOf("master-key", "wallets")).isEmpty())
    }

    @Test
    fun duplicateDriveFileNamesAreDetected() {
        assertEquals(
            setOf("master-key.json"),
            duplicateDriveFileNames(listOf("master-key.json", "wallet-record.json", "master-key.json")),
        )
        assertTrue(duplicateDriveFileNames(listOf("master-key.json", "wallet-record.json")).isEmpty())
    }

    @Test
    fun backupFileLocationsRejectDuplicateJsonFiles() {
        val error =
            runCatching {
                driveBackupFileLocations(
                    listOf("master-key.json", "notes.txt", "master-key.json"),
                )
            }.exceptionOrNull()

        assertTrue(error is DriveHttpException)
        assertEquals(HttpURLConnection.HTTP_CONFLICT, (error as DriveHttpException).statusCode)
        assertEquals("duplicate google drive file: master-key.json", error.body)
    }

    @Test
    fun backupFileLocationsIgnoreNonJsonFilesAndApplyLocation() {
        assertEquals(
            listOf("wallets/wallet-record.json"),
            driveBackupFileLocations(
                listOf("wallet-record.json", "notes.txt"),
                { fileName -> "wallets/$fileName" },
            ),
        )
    }

    @Test
    fun cloudBackupNamespaceValidationMatchesRustShape() {
        assertTrue(isValidCloudBackupNamespaceId("0123456789abcdef0123456789abcdef"))
        assertFalse(isValidCloudBackupNamespaceId("0123456789ABCDEF0123456789abcdef"))
        assertFalse(isValidCloudBackupNamespaceId("../0123456789abcdef0123456789abcd"))
        assertFalse(isValidCloudBackupNamespaceId("0123456789abcdef"))
    }

    @Test
    fun foregroundAuthorizationTimeoutMapsToAuthorizationRequired() {
        val error = mapDriveListError(ForegroundAuthorizationTimeoutException("google drive authorization timed out"))

        assertTrue(error is CloudStorageException.AuthorizationRequired)
        assertEquals(
            "google drive authorization timed out",
            (error as CloudStorageException.AuthorizationRequired).v1,
        )
    }

    @Test
    fun googleApiErrorsAreMappedToUnavailableWithStatus() {
        val apiError = ApiException(Status(CommonStatusCodes.DEVELOPER_ERROR))
        val uploadError = mapDriveUploadError(apiError, "wallet-record")
        val listError = mapDriveListError(apiError)

        assertTrue(uploadError is CloudStorageException.NotAvailable)
        assertTrue(listError is CloudStorageException.NotAvailable)
        assertEquals(
            "google drive is unavailable: DEVELOPER_ERROR",
            (uploadError as CloudStorageException.NotAvailable).v1,
        )
        assertEquals(
            "google drive is unavailable: DEVELOPER_ERROR",
            (listError as CloudStorageException.NotAvailable).v1,
        )
    }

    @Test
    fun unregisteredGoogleApiErrorsPointAtOAuthSetup() {
        val apiError = ApiException(
            Status(
                CommonStatusCodes.INTERNAL_ERROR,
                "[8] Unknown error [status=UNREGISTERED_ON_API_CONSOLE].",
            ),
        )
        val listError = mapDriveListError(apiError)

        assertTrue(listError is CloudStorageException.NotAvailable)
        assertEquals(
            "google drive is unavailable: google drive OAuth client is not registered for this app",
            (listError as CloudStorageException.NotAvailable).v1,
        )
    }

    @Test
    fun walletOperationErrorsUseLocationErrorId() =
        runBlocking {
            val storage = AndroidCloudStorageAccess(
                FailingDriveAuthorization(DriveHttpException(404, "missing")),
                TestDriveAccountBindingStore(),
            )
            val location = RemoteBackupLocation(relativePath = "wallets/wallet-record.json")

            val uploadError =
                captureError {
                    storage.uploadWalletBackup(
                        namespace = "namespace",
                        recordId = "record-id",
                        location = location,
                        data = byteArrayOf(),
                        policy = CloudAccessPolicy.SILENT,
                    )
                }
            val downloadError =
                captureError {
                    storage.downloadWalletBackup(
                        namespace = "namespace",
                        recordId = "record-id",
                        locations = listOf(location),
                        policy = CloudAccessPolicy.SILENT,
                    )
                }
            val deleteError =
                captureError {
                    storage.deleteWalletBackup(
                        namespace = "namespace",
                        recordId = "record-id",
                        locations = listOf(location),
                        policy = CloudAccessPolicy.SILENT,
                    )
                }

            assertNotFoundTarget(uploadError, "wallets/wallet-record.json")
            assertNotFoundTarget(downloadError, "wallets/wallet-record.json")
            assertNotFoundTarget(deleteError, "wallets/wallet-record.json")
        }

    @Test
    fun driveHttpUploadErrorsAreMappedBeforeGenericIoErrors() {
        val notFound = mapDriveUploadError(DriveHttpException(404, "missing"), "wallet-record")
        val quotaExceeded = mapDriveUploadError(DriveHttpException(429, "rate limit"), "wallet-record")
        val forbiddenQuota = mapDriveUploadError(
            DriveHttpException(403, driveErrorBody("storageQuotaExceeded")),
            "wallet-record",
        )
        val forbiddenAuthorization = mapDriveUploadError(
            DriveHttpException(403, driveErrorBody("insufficientFilePermissions")),
            "wallet-record",
        )
        val disabledApi = mapDriveUploadError(
            DriveHttpException(403, disabledDriveApiBody()),
            "wallet-record",
        )
        val offline = mapDriveUploadError(IOException("network unavailable"), "wallet-record")

        assertTrue(notFound is CloudStorageException.NotFound)
        assertTrue(quotaExceeded is CloudStorageException.QuotaExceeded)
        assertTrue(forbiddenQuota is CloudStorageException.QuotaExceeded)
        assertTrue(forbiddenAuthorization is CloudStorageException.AuthorizationRequired)
        assertTrue(disabledApi is CloudStorageException.NotAvailable)
        assertTrue(offline is CloudStorageException.Offline)
        assertEquals("wallet-record", (notFound as CloudStorageException.NotFound).v1)
        assertEquals(
            "google drive access was rejected",
            (forbiddenAuthorization as CloudStorageException.AuthorizationRequired).v1,
        )
        assertEquals(
            "google drive API is not enabled for this Google Cloud project",
            (disabledApi as CloudStorageException.NotAvailable).v1,
        )
        assertEquals("network unavailable", (offline as CloudStorageException.Offline).v1)
    }

    @Test
    fun driveHttpListErrorsAreMappedBeforeGenericIoErrors() {
        val notFound = mapDriveListError(DriveHttpException(404, "missing"))
        val quotaExceeded = mapDriveListError(DriveHttpException(429, "rate limit"))
        val forbiddenQuota = mapDriveListError(DriveHttpException(403, driveErrorBody("quotaExceeded")))
        val forbiddenAuthorization = mapDriveListError(
            DriveHttpException(403, driveErrorBody("insufficientFilePermissions")),
        )
        val disabledApi = mapDriveListError(DriveHttpException(403, disabledDriveApiBody()))
        val conflict = mapDriveListError(DriveHttpException(409, "duplicate google drive file: master-key.json"))
        val offline = mapDriveListError(IOException("network unavailable"))

        assertTrue(notFound is CloudStorageException.NotFound)
        assertTrue(quotaExceeded is CloudStorageException.QuotaExceeded)
        assertTrue(forbiddenQuota is CloudStorageException.QuotaExceeded)
        assertTrue(forbiddenAuthorization is CloudStorageException.AuthorizationRequired)
        assertTrue(disabledApi is CloudStorageException.NotAvailable)
        assertTrue(conflict is CloudStorageException.NotAvailable)
        assertTrue(offline is CloudStorageException.Offline)
        assertEquals("drive file", (notFound as CloudStorageException.NotFound).v1)
        assertEquals(
            "google drive access was rejected",
            (forbiddenAuthorization as CloudStorageException.AuthorizationRequired).v1,
        )
        assertEquals(
            "google drive API is not enabled for this Google Cloud project",
            (disabledApi as CloudStorageException.NotAvailable).v1,
        )
        assertEquals(
            "duplicate google drive file: master-key.json",
            (conflict as CloudStorageException.NotAvailable).v1,
        )
        assertEquals("network unavailable", (offline as CloudStorageException.Offline).v1)
    }

    @Test
    fun driveQuotaReasonsParsesStructuredDriveReasons() {
        assertEquals(
            setOf(DriveQuotaReason.StorageQuotaExceeded, DriveQuotaReason.UserRateLimitExceeded),
            driveQuotaReasons(
                """
                {
                    "error": {
                        "reason": "storageQuotaExceeded",
                        "errors": [
                            { "reason": "userRateLimitExceeded" },
                            { "reason": "insufficientFilePermissions" }
                        ]
                    }
                }
                """.trimIndent(),
            ),
        )
    }

    private suspend fun assertDriveAccountIdentityLookupClearsTokenAndRetries(
        statusCode: Int,
        body: String,
    ) {
        TestDriveServer().use { server ->
            server.enqueue(statusCode, body)
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
            server.enqueue(HttpURLConnection.HTTP_OK, """{"files":[]}""")

            val authorization = SequentialDriveAuthorization(listOf("token-1", "token-2"))
            val storage = AndroidCloudStorageAccess(
                driveAuthorization = authorization,
                accountBindingStore = TestDriveAccountBindingStore(),
                driveApiEndpoints = DriveApiEndpoints(
                    aboutEndpoint = "${server.baseUrl}/about",
                    filesEndpoint = "${server.baseUrl}/files",
                    uploadEndpoint = "${server.baseUrl}/upload",
                ),
                drivePathNamesProvider = { testDrivePathNames },
            )

            val namespacesResult = runCatching {
                storage.listNamespaces(CloudAccessPolicy.SILENT)
            }
            assertTrue(
                cloudStorageFailureMessage(namespacesResult.exceptionOrNull()),
                namespacesResult.isSuccess,
            )
            assertEquals(emptyList<String>(), namespacesResult.getOrNull())
            assertEquals(listOf(false, false), authorization.accessRequests)
            assertEquals(listOf("token-1"), authorization.clearedTokens)

            val requests = server.requests()
            assertEquals(listOf("/about", "/about", "/files"), requests.map { it.path.substringBefore("?") })
            assertEquals(
                listOf("Bearer token-1", "Bearer token-2", "Bearer token-2"),
                requests.map { it.authorization },
            )
        }
    }

    private fun cloudStorageFailureMessage(error: Throwable?): String =
        when (error) {
            null -> "listNamespaces succeeded"
            is CloudStorageException.AuthorizationRequired ->
                "listNamespaces failed with AuthorizationRequired: ${error.v1}"
            is CloudStorageException.NotAvailable ->
                "listNamespaces failed with NotAvailable: ${error.v1}"
            is CloudStorageException.NotFound ->
                "listNamespaces failed with NotFound: ${error.v1}"
            is CloudStorageException.Offline ->
                "listNamespaces failed with Offline: ${error.v1}"
            is CloudStorageException.QuotaExceeded ->
                "listNamespaces failed with QuotaExceeded"
            is CloudStorageException.UploadFailed ->
                "listNamespaces failed with UploadFailed: ${error.v1}"
            is CloudStorageException.DownloadFailed ->
                "listNamespaces failed with DownloadFailed: ${error.v1}"
            else -> "listNamespaces failed with ${error.javaClass.name}: ${error.message}"
        }

    private fun driveErrorBody(reason: String): String =
        """
        {
            "error": {
                "errors": [
                    { "reason": "$reason" }
                ]
            }
        }
        """.trimIndent()

    private fun disabledDriveApiBody(): String =
        """
        {
            "error": {
                "message": "Google Drive API has not been used in project 738970325901 before or it is disabled.",
                "errors": [
                    { "reason": "accessNotConfigured" }
                ],
                "details": [
                    { "reason": "SERVICE_DISABLED" }
                ]
            }
        }
        """.trimIndent()

    private suspend fun captureError(block: suspend () -> Unit): Throwable? =
        try {
            block()
            null
        } catch (error: Throwable) {
            error
        }

    private fun assertNotFoundTarget(error: Throwable?, expected: String) {
        assertTrue(error is CloudStorageException.NotFound)
        assertEquals(expected, (error as CloudStorageException.NotFound).v1)
    }

    private class FailingDriveAuthorization(
        private val error: Throwable,
    ) : DriveAuthorization {
        override suspend fun accessToken(interactive: Boolean): DriveAccessToken {
            throw error
        }

        override suspend fun clearToken(token: String) = Unit
    }

    private class SequentialDriveAuthorization(
        tokens: List<String>,
    ) : DriveAuthorization {
        private val tokens = ArrayDeque(tokens)
        val accessRequests = mutableListOf<Boolean>()
        val clearedTokens = mutableListOf<String>()

        override suspend fun accessToken(interactive: Boolean): DriveAccessToken {
            accessRequests.add(interactive)
            if (tokens.isEmpty()) {
                throw AssertionError("unexpected drive access token request")
            }

            return DriveAccessToken(token = tokens.removeFirst(), account = null)
        }

        override suspend fun clearToken(token: String) {
            clearedTokens.add(token)
        }
    }

    private class RecordingDriveAuthorization : DriveAuthorization {
        var token = "token-1"
        var account: DriveAccountIdentity? = DriveAccountIdentity(id = "account-1", email = "person@example.com")
        val accessRequests = mutableListOf<Boolean>()
        val clearedTokens = mutableListOf<String>()

        override suspend fun accessToken(interactive: Boolean): DriveAccessToken {
            accessRequests.add(interactive)
            return DriveAccessToken(token = token, account = account)
        }

        override suspend fun clearToken(token: String) {
            clearedTokens.add(token)
        }
    }

    private class BlockingClearDriveAuthorization : DriveAuthorization {
        var token = "token-1"
        var account = DriveAccountIdentity(id = "account-1", email = "person@example.com")
        val accessRequests = mutableListOf<Boolean>()
        val clearedTokens = mutableListOf<String>()
        val clearStarted = CompletableDeferred<Unit>()
        val finishClear = CompletableDeferred<Unit>()

        override suspend fun accessToken(interactive: Boolean): DriveAccessToken {
            accessRequests.add(interactive)
            return DriveAccessToken(token = token, account = account)
        }

        override suspend fun clearToken(token: String) {
            clearedTokens.add(token)
            clearStarted.complete(Unit)
            finishClear.await()
        }
    }

    private class TestDriveServer : AutoCloseable {
        private val serverSocket = ServerSocket(0)
        private val responseLock = Any()
        private val requestLock = Any()
        private val responses = ArrayDeque<TestDriveResponse>()
        private val requests = mutableListOf<TestDriveRequest>()

        @Volatile
        private var serverError: Throwable? = null

        private val thread =
            Thread({ serve() }, "test-drive-server")
                .apply {
                    isDaemon = true
                    start()
                }

        val baseUrl = "http://127.0.0.1:${serverSocket.localPort}"

        fun enqueue(statusCode: Int, body: String) {
            synchronized(responseLock) {
                responses.add(TestDriveResponse(statusCode, body))
            }
        }

        fun requests(): List<TestDriveRequest> =
            synchronized(requestLock) {
                requests.toList()
            }

        override fun close() {
            serverSocket.close()
            thread.join(1_000)
            serverError?.let { throw AssertionError("test drive server failed", it) }
        }

        private fun serve() {
            try {
                while (!serverSocket.isClosed) {
                    val socket =
                        try {
                            serverSocket.accept()
                        } catch (_: SocketException) {
                            return
                        }

                    socket.use {
                        val reader = BufferedReader(InputStreamReader(it.getInputStream(), Charsets.UTF_8))
                        val requestLine = reader.readLine() ?: return@use
                        val authorization = readAuthorizationHeader(reader)
                        val requestTarget = requestLine.split(" ").getOrNull(1).orEmpty()

                        synchronized(requestLock) {
                            requests.add(TestDriveRequest(path = requestTarget, authorization = authorization))
                        }

                        val response =
                            synchronized(responseLock) {
                                if (responses.isEmpty()) {
                                    TestDriveResponse(HttpURLConnection.HTTP_INTERNAL_ERROR, "unexpected request")
                                } else {
                                    responses.removeFirst()
                                }
                            }
                        val responseBody = response.body.toByteArray(Charsets.UTF_8)
                        val statusText = if (response.statusCode in 200..299) "OK" else "Error"
                        val headers =
                            "HTTP/1.1 ${response.statusCode} $statusText\r\n" +
                                "Content-Type: application/json\r\n" +
                                "Content-Length: ${responseBody.size}\r\n" +
                                "Connection: close\r\n" +
                                "\r\n"

                        it.getOutputStream().use { output ->
                            output.write(headers.toByteArray(Charsets.UTF_8))
                            output.write(responseBody)
                        }
                    }
                }
            } catch (error: Throwable) {
                if (!serverSocket.isClosed) {
                    serverError = error
                }
            }
        }

        private fun readAuthorizationHeader(reader: BufferedReader): String? {
            var authorization: String? = null
            while (true) {
                val line = reader.readLine() ?: return authorization
                if (line.isEmpty()) {
                    return authorization
                }

                val separator = line.indexOf(':')
                if (separator < 0) {
                    continue
                }

                val name = line.substring(0, separator)
                if (name.equals("Authorization", ignoreCase = true)) {
                    authorization = line.substring(separator + 1).trim()
                }
            }
        }
    }

    private data class TestDriveResponse(
        val statusCode: Int,
        val body: String,
    )

    private data class TestDriveRequest(
        val path: String,
        val authorization: String?,
    )

    private class TestDriveAccountBindingStore(
        private var identity: DriveAccountIdentity? = null,
    ) : DriveAccountBindingStore {
        override fun selectedIdentity(): DriveAccountIdentity? = identity

        override fun bindIdentity(identity: DriveAccountIdentity) {
            this.identity = identity
        }

        override fun clearIdentity() {
            identity = null
        }
    }
}
