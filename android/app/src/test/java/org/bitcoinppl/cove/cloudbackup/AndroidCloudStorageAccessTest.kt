package org.bitcoinppl.cove.cloudbackup

import com.google.android.gms.common.api.ApiException
import com.google.android.gms.common.api.CommonStatusCodes
import com.google.android.gms.common.api.Status
import java.io.IOException
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.async
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.test.runTest
import kotlinx.coroutines.yield
import org.bitcoinppl.cove_core.device.CloudAccessPolicy
import org.bitcoinppl.cove_core.device.CloudStorageException
import org.bitcoinppl.cove_core.device.RemoteBackupLocation
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class AndroidCloudStorageAccessTest {
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
    fun cachingDriveAuthorizationReusesTokenUntilCleared() =
        runBlocking {
            var now = 0L
            val delegate = RecordingDriveAuthorization()
            val authorization = CachingDriveAuthorization(
                delegate = delegate,
                elapsedRealtime = { now },
                cacheWindowMs = 1_000,
            )

            assertEquals("token-1", authorization.accessToken(interactive = false))
            assertEquals("token-1", authorization.accessToken(interactive = true))
            assertEquals(listOf(false), delegate.accessRequests)

            now = 500
            delegate.token = "token-2"
            authorization.clearToken("other-token")
            assertEquals("token-1", authorization.accessToken(interactive = false))
            assertEquals(listOf(false), delegate.accessRequests)

            authorization.clearToken("token-1")
            assertEquals("token-2", authorization.accessToken(interactive = false))
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

            assertEquals("token-1", authorization.accessToken(interactive = false))

            now = 999
            assertEquals("token-1", authorization.accessToken(interactive = false))

            now = 2_000
            delegate.token = "token-2"
            assertEquals("token-2", authorization.accessToken(interactive = false))
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

            assertEquals("token-1", authorization.accessToken(interactive = false))

            delegate.token = "token-2"
            val clear = async { authorization.clearToken("token-1") }
            delegate.clearStarted.await()

            val refresh = async { authorization.accessToken(interactive = false) }
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
            val storage = AndroidCloudStorageAccess(FailingDriveAuthorization(DriveHttpException(404, "missing")))
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
        val offline = mapDriveListError(IOException("network unavailable"))

        assertTrue(notFound is CloudStorageException.NotFound)
        assertTrue(quotaExceeded is CloudStorageException.QuotaExceeded)
        assertTrue(forbiddenQuota is CloudStorageException.QuotaExceeded)
        assertTrue(forbiddenAuthorization is CloudStorageException.AuthorizationRequired)
        assertTrue(disabledApi is CloudStorageException.NotAvailable)
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
        override suspend fun accessToken(interactive: Boolean): String {
            throw error
        }

        override suspend fun clearToken(token: String) = Unit
    }

    private class RecordingDriveAuthorization : DriveAuthorization {
        var token = "token-1"
        val accessRequests = mutableListOf<Boolean>()
        val clearedTokens = mutableListOf<String>()

        override suspend fun accessToken(interactive: Boolean): String {
            accessRequests.add(interactive)
            return token
        }

        override suspend fun clearToken(token: String) {
            clearedTokens.add(token)
        }
    }

    private class BlockingClearDriveAuthorization : DriveAuthorization {
        var token = "token-1"
        val accessRequests = mutableListOf<Boolean>()
        val clearedTokens = mutableListOf<String>()
        val clearStarted = CompletableDeferred<Unit>()
        val finishClear = CompletableDeferred<Unit>()

        override suspend fun accessToken(interactive: Boolean): String {
            accessRequests.add(interactive)
            return token
        }

        override suspend fun clearToken(token: String) {
            clearedTokens.add(token)
            clearStarted.complete(Unit)
            finishClear.await()
        }
    }
}
