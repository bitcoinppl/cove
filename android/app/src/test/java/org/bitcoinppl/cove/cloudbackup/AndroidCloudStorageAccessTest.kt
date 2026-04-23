package org.bitcoinppl.cove.cloudbackup

import kotlinx.coroutines.runBlocking
import org.bitcoinppl.cove_core.device.CloudAccessPolicy
import org.bitcoinppl.cove_core.device.CloudStorageException
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import java.io.IOException

class AndroidCloudStorageAccessTest {
    @Test
    fun createUploadMetadataIncludesParents() {
        val metadata = createUploadMetadata(fileName = "wallet-record.json", parentId = "folder-123")

        assertEquals("wallet-record.json", metadata.name)
        assertEquals(listOf("folder-123"), metadata.parents)
    }

    @Test
    fun overwriteUploadMetadataOmitsParents() {
        val metadata = overwriteUploadMetadata(fileName = "wallet-record.json")

        assertEquals("wallet-record.json", metadata.name)
        assertEquals(emptyList<String>(), metadata.parents)
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

    private class FailingDriveAuthorization(
        private val error: Throwable,
    ) : DriveAuthorization {
        override suspend fun accessToken(interactive: Boolean): String {
            throw error
        }

        override suspend fun clearToken(token: String) = Unit
    }
}
