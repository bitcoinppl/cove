package org.bitcoinppl.cove.cloudbackup

import com.google.android.gms.common.api.ApiException
import com.google.android.gms.common.api.CommonStatusCodes
import java.io.IOException
import java.net.HttpURLConnection
import java.net.SocketTimeoutException
import java.net.URLEncoder
import java.net.UnknownHostException
import java.nio.charset.StandardCharsets
import org.bitcoinppl.cove.Log
import org.bitcoinppl.cove_core.device.CloudStorageException
import org.bitcoinppl.cove_core.device.RemoteBackupLocation
import org.json.JSONArray
import org.json.JSONObject

internal data class DriveLocationParts(
    val parentFolders: List<String>,
    val fileName: String,
)

internal fun driveLocationParts(relativePath: String): DriveLocationParts {
    require(relativePath.isNotBlank()) { "relativePath must not be blank" }

    val parts = relativePath.split("/")
    require(parts.none { it.isBlank() || it == "." || it == ".." })

    return DriveLocationParts(
        parentFolders = parts.dropLast(1),
        fileName = parts.last(),
    )
}

internal val RemoteBackupLocation.parts: DriveLocationParts
    get() = driveLocationParts(relativePath)

internal val RemoteBackupLocation.fileName: String
    get() = parts.fileName

internal fun RemoteBackupLocation.errorId(fallback: String): String =
    relativePath.ifBlank { fallback }

internal data class UploadMetadata(
    val name: String,
    val parents: List<String> = emptyList(),
) {
    fun toJson(): JSONObject =
        JSONObject()
            .put("name", name)
            .apply {
                if (parents.isNotEmpty()) {
                    put("parents", JSONArray(parents))
                }
            }
}

internal fun createUploadMetadata(
    fileName: String,
    parentId: String,
): UploadMetadata =
    UploadMetadata(
        name = fileName,
        parents = listOf(parentId),
    )

internal fun overwriteUploadMetadata(fileName: String): UploadMetadata =
    UploadMetadata(name = fileName)

internal enum class DriveQuotaReason {
    StorageQuotaExceeded,
    QuotaExceeded,
    UserRateLimitExceeded,
    RateLimitExceeded,
    DailyLimitExceeded;

    companion object {
        fun from(value: String): DriveQuotaReason? =
            when (value) {
                "storageQuotaExceeded" -> StorageQuotaExceeded
                "quotaExceeded" -> QuotaExceeded
                "userRateLimitExceeded" -> UserRateLimitExceeded
                "rateLimitExceeded" -> RateLimitExceeded
                "dailyLimitExceeded" -> DailyLimitExceeded
                else -> null
            }
    }
}

internal data class DriveErrorInfo(
    val message: String?,
    val reasons: Set<String>,
) {
    fun hasReason(vararg expected: String): Boolean =
        expected.any(reasons::contains)
}

internal fun driveErrorInfo(body: String): DriveErrorInfo {
    val root = runCatching { JSONObject(body) }.getOrNull()
    val error = root?.optJSONObject("error")
        ?: return DriveErrorInfo(message = body.takeIf(String::isNotBlank), reasons = emptySet())
    val reasons = mutableSetOf<String>()

    error.optString("reason").takeIf(String::isNotBlank)?.let(reasons::add)

    val errors = error.optJSONArray("errors")
    if (errors != null) {
        for (index in 0 until errors.length()) {
            val reason = errors.optJSONObject(index)?.optString("reason") ?: continue
            reason.takeIf(String::isNotBlank)?.let(reasons::add)
        }
    }

    val details = error.optJSONArray("details")
    if (details != null) {
        for (index in 0 until details.length()) {
            val detail = details.optJSONObject(index) ?: continue
            detail.optString("reason").takeIf(String::isNotBlank)?.let(reasons::add)
        }
    }

    return DriveErrorInfo(
        message = error.optString("message").takeIf(String::isNotBlank),
        reasons = reasons,
    )
}

internal fun driveQuotaReasons(body: String): Set<DriveQuotaReason> {
    return driveErrorInfo(body).reasons.mapNotNullTo(mutableSetOf(), DriveQuotaReason::from)
}

private fun DriveHttpException.isQuotaExceeded(): Boolean =
    driveQuotaReasons(body).isNotEmpty()

private fun DriveHttpException.isGoogleDriveApiDisabled(): Boolean {
    val info = driveErrorInfo(body)
    return info.hasReason("accessNotConfigured", "serviceDisabled", "SERVICE_DISABLED") ||
        info.message?.contains("Google Drive API has not been used", ignoreCase = true) == true ||
        info.message?.contains("Google Drive API is disabled", ignoreCase = true) == true
}

internal fun DriveHttpException.isAuthorizationRejected(): Boolean {
    val info = driveErrorInfo(body)
    return info.hasReason(
        "insufficientPermissions",
        "insufficientFilePermissions",
        "appNotAuthorizedToFile",
    )
}

private fun DriveHttpException.driveMessage(fallback: String): String =
    driveErrorInfo(body).message?.takeIf(String::isNotBlank) ?: body.ifBlank { fallback }

private fun DriveHttpException.forbiddenError(fallback: String): CloudStorageException =
    when {
        isQuotaExceeded() -> CloudStorageException.QuotaExceeded()
        isGoogleDriveApiDisabled() ->
            CloudStorageException.NotAvailable(
                "google drive API is not enabled for this Google Cloud project",
            )
        isAuthorizationRejected() ->
            CloudStorageException.AuthorizationRequired("google drive access was rejected")
        else -> CloudStorageException.NotAvailable(driveMessage(fallback))
    }

private fun AuthorizationRequiredException.cloudStorageMessage(): String =
    message?.takeIf(String::isNotBlank) ?: "google drive authorization is required"

private fun DriveAccountBindingException.cloudStorageMessage(): String =
    message?.takeIf(String::isNotBlank) ?: "google drive account is unavailable"

private fun ForegroundAuthorizationTimeoutException.cloudStorageMessage(): String =
    message?.takeIf(String::isNotBlank) ?: "google drive authorization timed out"

private fun ApiException.cloudStorageMessage(prefix: String): String {
    val status = CommonStatusCodes.getStatusCodeString(statusCode)
    val details = message
        ?.trim()
        ?.takeIf { it.isNotBlank() && it != status && it != "$statusCode:" }

    if (details?.contains("UNREGISTERED_ON_API_CONSOLE") == true) {
        return "$prefix: google drive OAuth client is not registered for this app"
    }

    return if (details == null) "$prefix: $status" else "$prefix: $status: $details"
}

private fun driveIdentityLookupFailedMessage(error: Throwable): String =
    "google drive identity verification failed: ${error.message ?: "unknown error"}"

internal fun logDriveWarning(message: String, error: Throwable) {
    runCatching { Log.w("AndroidCloudStorage", message, error) }
}

internal fun logDriveWarning(message: String) {
    runCatching { Log.w("AndroidCloudStorage", message) }
}

internal fun logDriveDebug(message: String) {
    runCatching { Log.d("AndroidCloudStorage", message) }
}

internal fun monotonicTimeMs(): Long = System.nanoTime() / 1_000_000L

internal fun driveAccountIdentityFromAboutResponse(response: JSONObject): DriveAccountIdentity? =
    response
        .optJSONObject("user")
        ?.let { user ->
            val drivePermissionId = user.optString("permissionId").takeIf(String::isNotBlank)
            val email = user.optString("emailAddress").takeIf(String::isNotBlank)

            if (drivePermissionId == null && email == null) {
                null
            } else {
                DriveAccountIdentity(drivePermissionId = drivePermissionId, email = email)
            }
        }

internal fun duplicateDriveFolderNames(folderNames: List<String>): Set<String> =
    folderNames
        .groupingBy { it }
        .eachCount()
        .filterValues { it > 1 }
        .keys

internal fun duplicateDriveFileNames(fileNames: List<String>): Set<String> =
    fileNames
        .groupingBy { it }
        .eachCount()
        .filterValues { it > 1 }
        .keys

internal fun duplicateDriveFolderException(folderName: String): DriveHttpException =
    DriveHttpException(HttpURLConnection.HTTP_CONFLICT, "duplicate google drive folder: $folderName")

internal fun duplicateDriveFileException(fileName: String): DriveHttpException =
    DriveHttpException(HttpURLConnection.HTTP_CONFLICT, "duplicate google drive file: $fileName")

internal fun driveBackupFileLocations(
    fileNames: List<String>,
    locationForFileName: (String) -> String = { it },
): List<String> {
    val jsonFileNames = fileNames.filter { it.endsWith(".json") }
    val duplicates = duplicateDriveFileNames(jsonFileNames)
    if (duplicates.isNotEmpty()) {
        throw duplicateDriveFileException(duplicates.first())
    }

    return jsonFileNames.map(locationForFileName)
}

internal fun isValidCloudBackupNamespaceId(namespace: String): Boolean =
    namespace.length == 32 && namespace.all { it in '0'..'9' || it in 'a'..'f' }

internal fun mapDriveUploadError(error: Throwable, target: String): CloudStorageException =
    when (error) {
        is DriveAccountBindingException ->
            CloudStorageException.AuthorizationRequired(error.cloudStorageMessage())
        is DriveAccountIdentityLookupException ->
            mapDriveIdentityLookupError(error)
        is ForegroundAuthorizationTimeoutException ->
            CloudStorageException.AuthorizationRequired(error.cloudStorageMessage())
        is AuthorizationRequiredException ->
            CloudStorageException.AuthorizationRequired(error.cloudStorageMessage())
        is ApiException ->
            if (error.statusCode == CommonStatusCodes.CANCELED) {
                CloudStorageException.AuthorizationRequired("google drive authorization was cancelled")
            } else {
                CloudStorageException.NotAvailable(error.cloudStorageMessage("google drive is unavailable"))
            }
        is DriveHttpException ->
            when (error.statusCode) {
                HttpURLConnection.HTTP_UNAUTHORIZED ->
                    CloudStorageException.AuthorizationRequired("google drive authorization is required")
                HttpURLConnection.HTTP_NOT_FOUND -> CloudStorageException.NotFound(target)
                HTTP_TOO_MANY_REQUESTS -> CloudStorageException.QuotaExceeded()
                HttpURLConnection.HTTP_FORBIDDEN ->
                    error.forbiddenError("drive upload was rejected")
                else -> CloudStorageException.UploadFailed(error.body.ifBlank { "drive upload failed" })
            }
        is UnknownHostException, is SocketTimeoutException, is IOException ->
            CloudStorageException.Offline(error.message ?: "offline")
        else -> CloudStorageException.UploadFailed(error.message ?: "drive upload failed")
    }

internal fun mapDriveDownloadError(error: Throwable, target: String): CloudStorageException =
    when (val mapped = mapDriveUploadError(error, target)) {
        is CloudStorageException.UploadFailed ->
            CloudStorageException.DownloadFailed(mapped.v1)
        else -> mapped
    }

internal fun mapDriveDeleteError(error: Throwable, target: String): CloudStorageException =
    when (val mapped = mapDriveUploadError(error, target)) {
        is CloudStorageException.UploadFailed ->
            CloudStorageException.NotAvailable(mapped.v1)
        else -> mapped
    }

internal fun mapDriveListError(error: Throwable): CloudStorageException =
    when (error) {
        is DriveAccountBindingException ->
            CloudStorageException.AuthorizationRequired(error.cloudStorageMessage())
        is DriveAccountIdentityLookupException ->
            mapDriveIdentityLookupError(error)
        is ForegroundAuthorizationTimeoutException ->
            CloudStorageException.AuthorizationRequired(error.cloudStorageMessage())
        is AuthorizationRequiredException ->
            CloudStorageException.AuthorizationRequired(error.cloudStorageMessage())
        is ApiException ->
            if (error.statusCode == CommonStatusCodes.CANCELED) {
                CloudStorageException.AuthorizationRequired("google drive authorization was cancelled")
            } else {
                CloudStorageException.NotAvailable(error.cloudStorageMessage("google drive is unavailable"))
            }
        is DriveHttpException ->
            when (error.statusCode) {
                HttpURLConnection.HTTP_UNAUTHORIZED ->
                    CloudStorageException.AuthorizationRequired("google drive authorization is required")
                HTTP_TOO_MANY_REQUESTS -> CloudStorageException.QuotaExceeded()
                HttpURLConnection.HTTP_CONFLICT ->
                    CloudStorageException.NotAvailable(
                        error.body.ifBlank { "conflicting google drive backup data" },
                    )
                HttpURLConnection.HTTP_NOT_FOUND -> CloudStorageException.NotFound("drive file")
                HttpURLConnection.HTTP_FORBIDDEN ->
                    error.forbiddenError("drive listing was rejected")
                else -> CloudStorageException.NotAvailable(error.body.ifBlank { "drive listing failed" })
            }
        is UnknownHostException, is SocketTimeoutException, is IOException ->
            CloudStorageException.Offline(error.message ?: "offline")
        else -> CloudStorageException.NotAvailable(error.message ?: "drive listing failed")
    }

internal class DriveHttpException(
    val statusCode: Int,
    val body: String,
) : IOException("drive request failed with status=$statusCode")

internal class DriveAccountIdentityLookupException(
    cause: Throwable,
) : IOException(driveIdentityLookupFailedMessage(cause), cause)

private fun mapDriveIdentityLookupError(error: DriveAccountIdentityLookupException): CloudStorageException {
    val cause = error.cause ?: return CloudStorageException.NotAvailable(
        error.message ?: "google drive identity verification failed",
    )

    return when (cause) {
        is DriveHttpException ->
            when (cause.statusCode) {
                HttpURLConnection.HTTP_UNAUTHORIZED ->
                    CloudStorageException.AuthorizationRequired(
                        "google drive identity verification requires authorization",
                    )
                HTTP_TOO_MANY_REQUESTS -> CloudStorageException.QuotaExceeded()
                HttpURLConnection.HTTP_FORBIDDEN -> cause.identityLookupForbiddenError()
                else -> CloudStorageException.NotAvailable(cause.identityLookupMessage())
            }
        is UnknownHostException, is SocketTimeoutException, is IOException ->
            CloudStorageException.Offline(cause.message ?: "offline")
        else -> CloudStorageException.NotAvailable(driveIdentityLookupFailedMessage(cause))
    }
}

private fun DriveHttpException.identityLookupForbiddenError(): CloudStorageException =
    when {
        isQuotaExceeded() -> CloudStorageException.QuotaExceeded()
        isGoogleDriveApiDisabled() ->
            CloudStorageException.NotAvailable(
                "google drive API is not enabled for this Google Cloud project",
            )
        isAuthorizationRejected() ->
            CloudStorageException.AuthorizationRequired("google drive identity verification was rejected")
        else -> CloudStorageException.NotAvailable(identityLookupMessage())
    }

private fun DriveHttpException.identityLookupMessage(): String =
    "google drive identity verification failed: ${driveMessage("drive identity lookup failed")}"

internal data class DriveApiEndpoints(
    val aboutEndpoint: String = DRIVE_ABOUT_ENDPOINT,
    val filesEndpoint: String = DRIVE_FILES_ENDPOINT,
    val uploadEndpoint: String = DRIVE_UPLOAD_ENDPOINT,
)

internal fun driveApiUrl(
    endpoint: String,
    queryParameters: List<Pair<String, String>>,
): String {
    if (queryParameters.isEmpty()) {
        return endpoint
    }

    val fragmentIndex = endpoint.indexOf('#')
    val endpointWithoutFragment = if (fragmentIndex == -1) endpoint else endpoint.substring(0, fragmentIndex)
    val fragment = if (fragmentIndex == -1) "" else endpoint.substring(fragmentIndex)
    val separator = if (endpointWithoutFragment.contains("?")) "&" else "?"
    val query =
        queryParameters.joinToString("&") { (name, value) ->
            "${driveQueryParameter(name)}=${driveQueryParameter(value)}"
        }

    return "$endpointWithoutFragment$separator$query$fragment"
}

private fun driveQueryParameter(value: String): String =
    URLEncoder
        .encode(value, StandardCharsets.UTF_8)
        .replace("+", "%20")

private object UnboundDriveAccountTokenCacheKey

internal fun driveAccountTokenCacheKey(store: DriveAccountBindingStore): Any =
    store.selectedIdentity() ?: UnboundDriveAccountTokenCacheKey

