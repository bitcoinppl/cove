package org.bitcoinppl.cove.cloudbackup

import com.google.android.gms.common.api.ApiException
import com.google.android.gms.common.api.CommonStatusCodes
import com.google.android.gms.common.api.Status
import java.io.IOException
import java.net.HttpURLConnection
import org.bitcoinppl.cove_core.device.CloudStorageException
import org.json.JSONObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class DriveHttpClientTest {
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
    fun driveAboutResponseProvidesAccountIdentity() {
        val response =
            JSONObject(
                """
                {
                    "user": {
                        "emailAddress": "Person@Example.com",
                        "permissionId": "account-1"
                    }
                }
                """.trimIndent(),
            )

        assertEquals(
            DriveAccountIdentity(drivePermissionId = "account-1", email = "person@example.com"),
            driveAccountIdentityFromAboutResponse(response),
        )
    }

    @Test
    fun driveAboutResponseOmitsBlankIdentity() {
        assertEquals(null, driveAccountIdentityFromAboutResponse(JSONObject("{}")))
        assertEquals(
            null,
            driveAccountIdentityFromAboutResponse(
                JSONObject(
                    """
                    {
                        "user": {
                            "emailAddress": " ",
                            "permissionId": ""
                        }
                    }
                    """.trimIndent(),
                ),
            ),
        )
    }

    @Test
    fun driveApiUrlEncodesQueryParameters() {
        assertEquals(
            "https://example.test/files?q=name%20%3D%20%27a%20b%2Bc%27&email=a%2Bb%40example.com",
            driveApiUrl(
                "https://example.test/files",
                listOf("q" to "name = 'a b+c'", "email" to "a+b@example.com"),
            ),
        )
    }

    @Test
    fun driveApiUrlKeepsExistingQueryAndFragmentOrder() {
        assertEquals(
            "https://example.test/files?alt=json&fields=files%28id%2Cname%29#metadata",
            driveApiUrl(
                "https://example.test/files?alt=json#metadata",
                listOf("fields" to "files(id,name)"),
            ),
        )
        assertEquals(
            "https://example.test/files#metadata",
            driveApiUrl("https://example.test/files#metadata", emptyList()),
        )
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
}
