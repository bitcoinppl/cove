package org.bitcoinppl.cove.cloudbackup

import java.io.IOException
import kotlinx.coroutines.runBlocking
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
    fun tokenAcquisitionFailuresAreMappedToCloudStorageExceptions() =
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
                "google drive authorization is required",
                (error as CloudStorageException.AuthorizationRequired).v1,
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
        val offline = mapDriveUploadError(IOException("network unavailable"), "wallet-record")

        assertTrue(notFound is CloudStorageException.NotFound)
        assertTrue(quotaExceeded is CloudStorageException.QuotaExceeded)
        assertTrue(forbiddenQuota is CloudStorageException.QuotaExceeded)
        assertTrue(forbiddenAuthorization is CloudStorageException.AuthorizationRequired)
        assertTrue(offline is CloudStorageException.Offline)
        assertEquals("wallet-record", (notFound as CloudStorageException.NotFound).v1)
        assertEquals(
            "google drive access was rejected",
            (forbiddenAuthorization as CloudStorageException.AuthorizationRequired).v1,
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
        val offline = mapDriveListError(IOException("network unavailable"))

        assertTrue(notFound is CloudStorageException.NotFound)
        assertTrue(quotaExceeded is CloudStorageException.QuotaExceeded)
        assertTrue(forbiddenQuota is CloudStorageException.QuotaExceeded)
        assertTrue(forbiddenAuthorization is CloudStorageException.AuthorizationRequired)
        assertTrue(offline is CloudStorageException.Offline)
        assertEquals("drive file", (notFound as CloudStorageException.NotFound).v1)
        assertEquals(
            "google drive access was rejected",
            (forbiddenAuthorization as CloudStorageException.AuthorizationRequired).v1,
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
}
