package org.bitcoinppl.cove.cloudbackup

import android.content.Context
import com.google.android.gms.common.api.ApiException
import com.google.android.gms.common.api.CommonStatusCodes
import java.io.ByteArrayOutputStream
import java.io.IOException
import java.net.HttpURLConnection
import java.net.SocketTimeoutException
import java.net.URLEncoder
import java.net.URL
import java.net.UnknownHostException
import java.nio.charset.StandardCharsets
import java.util.concurrent.ConcurrentHashMap
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.coroutines.withContext
import org.bitcoinppl.cove.Log
import org.bitcoinppl.cove_core.device.CloudAccessPolicy
import org.bitcoinppl.cove_core.device.CloudStorageAccess
import org.bitcoinppl.cove_core.device.CloudStorageException
import org.bitcoinppl.cove_core.device.CloudSyncHealth
import org.bitcoinppl.cove_core.device.RemoteBackupLocation
import org.bitcoinppl.cove_core.device.cloudBackupLocationsSyncHealth
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

private val RemoteBackupLocation.parts: DriveLocationParts
    get() = driveLocationParts(relativePath)

private val RemoteBackupLocation.fileName: String
    get() = parts.fileName

private fun RemoteBackupLocation.errorId(fallback: String): String =
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

private fun DriveHttpException.isAuthorizationRejected(): Boolean {
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

private fun logDriveWarning(message: String, error: Throwable) {
    runCatching { Log.w("AndroidCloudStorage", message, error) }
}

private fun logDriveWarning(message: String) {
    runCatching { Log.w("AndroidCloudStorage", message) }
}

private fun logDriveDebug(message: String) {
    runCatching { Log.d("AndroidCloudStorage", message) }
}

internal fun monotonicTimeMs(): Long = System.nanoTime() / 1_000_000L

internal fun driveAccountIdentityFromAboutResponse(response: JSONObject): DriveAccountIdentity? =
    response
        .optJSONObject("user")
        ?.optString("emailAddress")
        ?.takeIf(String::isNotBlank)
        ?.let { email -> DriveAccountIdentity(id = null, email = email) }

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

private fun duplicateDriveFolderException(folderName: String): DriveHttpException =
    DriveHttpException(HttpURLConnection.HTTP_CONFLICT, "duplicate google drive folder: $folderName")

private fun duplicateDriveFileException(fileName: String): DriveHttpException =
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

internal data class DriveApiEndpoints(
    val aboutEndpoint: String = DRIVE_ABOUT_ENDPOINT,
    val filesEndpoint: String = DRIVE_FILES_ENDPOINT,
    val uploadEndpoint: String = DRIVE_UPLOAD_ENDPOINT,
)

private fun driveApiUrl(
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

class AndroidCloudStorageAccess internal constructor(
    private val driveAuthorization: DriveAuthorization,
    private val accountBindingStore: DriveAccountBindingStore,
    private val driveApiEndpoints: DriveApiEndpoints = DriveApiEndpoints(),
    drivePathNamesProvider: () -> DrivePathNames = { DrivePaths.defaultNames },
) : CloudStorageAccess {
    constructor(context: Context) : this(
        SharedPreferencesDriveAccountBindingStore(context.applicationContext),
    )

    private constructor(accountBindingStore: SharedPreferencesDriveAccountBindingStore) : this(
        CachingDriveAuthorization(
            DriveAuthorizationHelper(accountBindingStore.appContext) {
                accountBindingStore.selectedIdentity()
            },
            cacheKey = { accountBindingStore.selectedIdentity() },
        ),
        accountBindingStore,
    )

    private val namespacesRootFolderMutex = Mutex()
    private val namespaceFolderMutexes = ConcurrentHashMap<String, Mutex>()
    private val childFolderMutexes = ConcurrentHashMap<String, Mutex>()
    private val drivePathNames: DrivePathNames by lazy(drivePathNamesProvider)

    private fun CloudAccessPolicy.allowsConsent(): Boolean =
        this == CloudAccessPolicy.CONSENT_ALLOWED

    override suspend fun uploadMasterKeyBackup(
        namespace: String,
        location: RemoteBackupLocation,
        data: ByteArray,
        policy: CloudAccessPolicy,
    ) {
        runDriveOperation(
            interactive = policy.allowsConsent(),
            onError = { error ->
                mapDriveUploadError(error, location.errorId("master key backup"))
            },
            bindAccountOnSuccess = { true },
        ) { token ->
            val namespaceFolderId = ensureNamespaceFolderId(token, namespace)
            val parentId = ensureLocationParentFolderId(token, namespaceFolderId, location)
            upsertFile(
                token = token,
                parentId = parentId,
                fileName = location.fileName,
                data = data,
            )
        }
    }

    override suspend fun uploadWalletBackup(
        namespace: String,
        recordId: String,
        location: RemoteBackupLocation,
        data: ByteArray,
        policy: CloudAccessPolicy,
    ) {
        runDriveOperation(
            interactive = policy.allowsConsent(),
            onError = { error -> mapDriveUploadError(error, location.errorId(recordId)) },
            bindAccountOnSuccess = { true },
        ) { token ->
            val namespaceFolderId = ensureNamespaceFolderId(token, namespace)
            val parentId = ensureLocationParentFolderId(token, namespaceFolderId, location)
            upsertFile(
                token = token,
                parentId = parentId,
                fileName = location.fileName,
                data = data,
            )
        }
    }

    override suspend fun downloadMasterKeyBackup(
        namespace: String,
        locations: List<RemoteBackupLocation>,
        policy: CloudAccessPolicy,
    ): ByteArray =
        runDriveOperation(
            interactive = policy.allowsConsent(),
            onError = { error ->
                val errorId = locations.firstOrNull()?.errorId("master key backup") ?: "master key backup"
                mapDriveDownloadError(error, errorId)
            },
        ) { token ->
            val namespaceFolderId = requireNamespaceFolderId(token, namespace)
            val fileId =
                findFileAtLocations(
                    token = token,
                    namespaceFolderId = namespaceFolderId,
                    locations = locations,
                )?.id ?: throw DriveHttpException(404, "master key backup not found")
            downloadFile(token, fileId)
        }

    override suspend fun downloadWalletBackup(
        namespace: String,
        recordId: String,
        locations: List<RemoteBackupLocation>,
        policy: CloudAccessPolicy,
    ): ByteArray =
        runDriveOperation(
            interactive = policy.allowsConsent(),
            onError = { error ->
                val errorId = locations.firstOrNull()?.errorId(recordId) ?: recordId
                mapDriveDownloadError(error, errorId)
            },
            bindAccountOnSuccess = { true },
        ) { token ->
            val namespaceFolderId = requireNamespaceFolderId(token, namespace)
            val fileId =
                findFileAtLocations(
                    token = token,
                    namespaceFolderId = namespaceFolderId,
                    locations = locations,
                )?.id ?: throw DriveHttpException(404, "wallet backup not found")
            downloadFile(token, fileId)
        }

    override suspend fun deleteWalletBackup(
        namespace: String,
        recordId: String,
        locations: List<RemoteBackupLocation>,
        policy: CloudAccessPolicy,
    ) {
        runDriveOperation(
            interactive = policy.allowsConsent(),
            onError = { error ->
                val errorId = locations.firstOrNull()?.errorId(recordId) ?: recordId
                mapDriveDeleteError(error, errorId)
            },
        ) { token ->
            val namespaceFolderId = requireNamespaceFolderId(token, namespace)
            val files =
                findFilesAtLocations(
                    token = token,
                    namespaceFolderId = namespaceFolderId,
                    locations = locations,
                )

            if (files.isEmpty()) {
                throw DriveHttpException(404, "wallet backup not found")
            }

            val failures = mutableListOf<DriveDeleteFailure>()
            files.forEach { file ->
                try {
                    driveRequest(
                        token = token,
                        method = "DELETE",
                        url = "${driveApiEndpoints.filesEndpoint}/${file.id}",
                    )
                } catch (error: Throwable) {
                    if (error is CancellationException) throw error
                    logDriveWarning("failed to delete drive backup file", error)
                    failures.add(DriveDeleteFailure(fileId = file.id, error = error))
                }
            }

            if (failures.isNotEmpty()) {
                throw aggregateDeleteFailures(failures)
            }
        }
    }

    override suspend fun deleteNamespace(
        namespace: String,
        policy: CloudAccessPolicy,
    ) {
        runDriveOperation(
            interactive = policy.allowsConsent(),
            onError = { error -> mapDriveDeleteError(error, namespace) },
        ) { token ->
            val mutex = namespaceFolderMutexes.computeIfAbsent(namespace) { Mutex() }
            mutex.withLock {
                val namespaceFolderId =
                    findNamespaceFolderId(token, namespace)
                        ?: throw DriveHttpException(404, "namespace not found")

                driveRequest(
                    token = token,
                    method = "DELETE",
                    url = "${driveApiEndpoints.filesEndpoint}/$namespaceFolderId",
                )
            }
        }
    }

    override suspend fun listNamespaces(policy: CloudAccessPolicy): List<String> =
        listNamespaces(interactive = policy.allowsConsent())

    override suspend fun listWalletFiles(
        namespace: String,
        policy: CloudAccessPolicy,
    ): List<String> =
        listWalletFiles(namespace, interactive = policy.allowsConsent())

    override suspend fun isBackupUploaded(
        namespace: String,
        recordId: String,
        locations: List<RemoteBackupLocation>,
        policy: CloudAccessPolicy,
    ): Boolean =
        runDriveOperation(
            interactive = policy.allowsConsent(),
            onError = { error -> mapDriveListError(error) },
            bindAccountOnSuccess = { uploaded -> uploaded },
        ) { token ->
            val namespaceFolderId = findNamespaceFolderId(token, namespace) ?: return@runDriveOperation false
            findFileAtLocations(token, namespaceFolderId, locations) != null
        }

    override suspend fun overallSyncHealth(policy: CloudAccessPolicy): CloudSyncHealth =
        try {
            runDriveOperation(
                interactive = policy.allowsConsent(),
                onError = { error -> throw error },
            ) { token ->
                cloudBackupLocationsSyncHealth(listNamespaceBackupLocations(token))
            }
        } catch (error: Throwable) {
            if (error is CancellationException) throw error
            val mapped = mapDriveListError(error)
            when (mapped) {
                is CloudStorageException.AuthorizationRequired ->
                    CloudSyncHealth.AuthorizationRequired(mapped.message)
                is CloudStorageException.Offline -> CloudSyncHealth.Failed(mapped.message)
                is CloudStorageException.NotAvailable -> CloudSyncHealth.Unavailable
                else -> CloudSyncHealth.Failed(mapped.message ?: "drive sync status failed")
            }
        }

    private suspend fun findNamespacesRootFolderId(token: String): String? =
        findChildByName(
            token = token,
            parentId = APP_DATA_FOLDER,
            fileName = drivePathNames.namespacesRootFolderName,
            foldersOnly = true,
        )?.id

    private suspend fun ensureNamespacesRootFolderId(token: String): String =
        namespacesRootFolderMutex.withLock {
            findNamespacesRootFolderId(token)
                ?: run {
                    val createdId =
                        createFolder(
                            token = token,
                            parentId = APP_DATA_FOLDER,
                            folderName = drivePathNames.namespacesRootFolderName,
                        )
                    findNamespacesRootFolderId(token) ?: createdId
                }
        }

    private suspend fun ensureNamespaceFolderId(
        token: String,
        namespace: String,
    ): String {
        val rootId = ensureNamespacesRootFolderId(token)
        val mutex = namespaceFolderMutexes.computeIfAbsent(namespace) { Mutex() }
        return mutex.withLock {
            findChildByName(
                token = token,
                parentId = rootId,
                fileName = namespace,
                foldersOnly = true,
            )?.id ?: run {
                val createdId = createFolder(token, rootId, namespace)
                findChildByName(
                    token = token,
                    parentId = rootId,
                    fileName = namespace,
                    foldersOnly = true,
                )?.id ?: createdId
            }
        }
    }

    private suspend fun findNamespaceFolderId(
        token: String,
        namespace: String,
    ): String? {
        val rootId = findNamespacesRootFolderId(token) ?: return null
        return findChildByName(
            token = token,
            parentId = rootId,
            fileName = namespace,
            foldersOnly = true,
        )?.id
    }

    private suspend fun requireNamespaceFolderId(
        token: String,
        namespace: String,
    ): String {
        val rootId =
            findNamespacesRootFolderId(token)
                ?: throw DriveHttpException(404, "namespaces root not found")
        return findChildByName(
            token = token,
            parentId = rootId,
            fileName = namespace,
            foldersOnly = true,
        )?.id ?: throw DriveHttpException(404, "namespace not found")
    }

    private suspend fun ensureChildFolderId(
        token: String,
        parentId: String,
        folderName: String,
    ): String {
        val mutex = childFolderMutexes.computeIfAbsent("$parentId/$folderName") { Mutex() }
        return mutex.withLock {
            findChildByName(
                token = token,
                parentId = parentId,
                fileName = folderName,
                foldersOnly = true,
            )?.id ?: run {
                val createdId = createFolder(token, parentId, folderName)
                findChildByName(
                    token = token,
                    parentId = parentId,
                    fileName = folderName,
                    foldersOnly = true,
                )?.id ?: createdId
            }
        }
    }

    private suspend fun ensureLocationParentFolderId(
        token: String,
        namespaceFolderId: String,
        location: RemoteBackupLocation,
    ): String {
        var parentId = namespaceFolderId
        for (folderName in location.parts.parentFolders) {
            parentId =
                ensureChildFolderId(
                    token = token,
                    parentId = parentId,
                    folderName = folderName,
                )
        }

        return parentId
    }

    private suspend fun findFilesAtLocations(
        token: String,
        namespaceFolderId: String,
        locations: List<RemoteBackupLocation>,
    ): List<DriveFileMetadata> =
        locations
            .mapNotNull { location ->
                findFileAtLocation(
                    token = token,
                    namespaceFolderId = namespaceFolderId,
                    location = location,
                )
            }
            .distinctBy { it.id }

    private suspend fun findFileAtLocations(
        token: String,
        namespaceFolderId: String,
        locations: List<RemoteBackupLocation>,
    ): DriveFileMetadata? =
        locations.firstNotNullOfOrNull { location ->
            findFileAtLocation(
                token = token,
                namespaceFolderId = namespaceFolderId,
                location = location,
            )
        }

    private suspend fun findFileAtLocation(
        token: String,
        namespaceFolderId: String,
        location: RemoteBackupLocation,
    ): DriveFileMetadata? {
        val parts = location.parts
        var parentId = namespaceFolderId
        for (folderName in parts.parentFolders) {
            parentId =
                findChildByName(
                    token = token,
                    parentId = parentId,
                    fileName = folderName,
                    foldersOnly = true,
                )?.id ?: return null
        }

        return findChildByName(
            token = token,
            parentId = parentId,
            fileName = parts.fileName,
        )
    }

    private suspend fun createFolder(
        token: String,
        parentId: String,
        folderName: String,
    ): String {
        val metadata =
            JSONObject()
                .put("name", folderName)
                .put("mimeType", DriveApi.FOLDER_MIME_TYPE)
                .put("parents", JSONArray().put(parentId))

        val response =
            driveRequest(
                token = token,
                method = "POST",
                url = driveApiEndpoints.filesEndpoint,
                body = metadata.toString().toByteArray(),
                contentType = "application/json; charset=utf-8",
            ).asJsonObject()

        return response.getString("id")
    }

    private suspend fun upsertFile(
        token: String,
        parentId: String,
        fileName: String,
        data: ByteArray,
    ) {
        val existing =
            findChildByName(
                token = token,
                parentId = parentId,
                fileName = fileName,
            )

        val metadata =
            if (existing == null) {
                createUploadMetadata(fileName, parentId).toJson()
            } else {
                overwriteUploadMetadata(fileName).toJson()
            }

        val boundary = "cove-${System.currentTimeMillis()}"
        val body = buildMultipartBody(boundary, metadata, data)
        val url =
            if (existing == null) {
                "${driveApiEndpoints.uploadEndpoint}?uploadType=multipart"
            } else {
                "${driveApiEndpoints.uploadEndpoint}/${existing.id}?uploadType=multipart"
            }

        driveRequest(
            token = token,
            method = if (existing == null) "POST" else "PATCH",
            url = url,
            body = body,
            contentType = "multipart/related; boundary=$boundary",
        )
    }

    private suspend fun listWalletFiles(
        namespace: String,
        interactive: Boolean,
    ): List<String> =
        runDriveOperation(
            interactive = interactive,
            onError = { error -> mapDriveListError(error) },
            bindAccountOnSuccess = { true },
        ) { token ->
            val namespaceFolderId = requireNamespaceFolderId(token, namespace)
            listBackupLocations(
                token = token,
                namespaceFolderId = namespaceFolderId,
            ).filter(drivePathNames::isWalletFile)
        }

    private suspend fun listBackupLocations(
        token: String,
        namespaceFolderId: String,
    ): List<String> {
        val immediateChildren =
            listChildren(
                token = token,
                parentId = namespaceFolderId,
                foldersOnly = false,
            )

        val locations =
            immediateChildren
                .backupFileLocations { it }
                .toMutableList()

        immediateChildren
            .singleFolderChild(drivePathNames.masterKeyFolderName)
            ?.let { masterKeyFolder ->
                listChildren(
                    token = token,
                    parentId = masterKeyFolder.id,
                    foldersOnly = false,
                ).backupFileLocations { "${drivePathNames.masterKeyFolderName}/$it" }
                    .let(locations::addAll)
            }

        immediateChildren
            .singleFolderChild(drivePathNames.walletsFolderName)
            ?.let { walletsFolder ->
                listChildren(
                    token = token,
                    parentId = walletsFolder.id,
                    foldersOnly = false,
                ).backupFileLocations(drivePathNames::walletLocationForFileName)
                    .let(locations::addAll)
            }

        return locations
    }

    private suspend fun listNamespaceBackupLocations(token: String): List<List<String>> {
        val namespacesRootId = findNamespacesRootFolderId(token) ?: return emptyList()

        return listNamespaceFolders(token, namespacesRootId)
            .filter { namespace -> isValidCloudBackupNamespaceId(namespace.name) }
            .map { namespace ->
                listBackupLocations(
                    token = token,
                    namespaceFolderId = namespace.id,
                )
            }
    }

    private suspend fun listNamespaceFolders(
        token: String,
        namespacesRootId: String,
    ): List<DriveFileMetadata> {
        val namespaces =
            listChildren(
                token = token,
                parentId = namespacesRootId,
                foldersOnly = true,
            )
        val duplicates = duplicateDriveFolderNames(namespaces.map { it.name })
        if (duplicates.isNotEmpty()) {
            throw duplicateDriveFolderException("namespace")
        }

        return namespaces
    }

    private suspend fun listNamespaces(
        interactive: Boolean,
    ): List<String> =
        runDriveOperation(
            interactive = interactive,
            onError = { error -> mapDriveListError(error) },
        ) { token ->
            val namespacesRootId = findNamespacesRootFolderId(token) ?: return@runDriveOperation emptyList()
            listNamespaceFolders(token, namespacesRootId).map { it.name }
        }

    private fun buildMultipartBody(
        boundary: String,
        metadata: JSONObject,
        data: ByteArray,
    ): ByteArray {
        val output = ByteArrayOutputStream()
        val prefix = "--$boundary\r\n"
        output.write(prefix.toByteArray())
        output.write("Content-Type: application/json; charset=UTF-8\r\n\r\n".toByteArray())
        output.write(metadata.toString().toByteArray())
        output.write("\r\n--$boundary\r\n".toByteArray())
        output.write("Content-Type: application/octet-stream\r\n\r\n".toByteArray())
        output.write(data)
        output.write("\r\n--$boundary--\r\n".toByteArray())
        return output.toByteArray()
    }

    private suspend fun downloadFile(
        token: String,
        fileId: String,
    ): ByteArray =
        driveRequest(
            token = token,
            method = "GET",
            url = "${driveApiEndpoints.filesEndpoint}/$fileId?alt=media",
        ).body

    private suspend fun listChildren(
        token: String,
        parentId: String,
        foldersOnly: Boolean,
    ): List<DriveFileMetadata> {
        val query =
            buildString {
                append("'")
                append(parentId)
                append("' in parents and trashed = false")
                if (foldersOnly) {
                    append(" and mimeType = '")
                    append(DriveApi.FOLDER_MIME_TYPE)
                    append("'")
                }
            }

        val children = mutableListOf<DriveFileMetadata>()
        var pageToken: String? = null

        do {
            val parameters =
                buildList {
                    add("spaces" to APP_DATA_SPACE)
                    add("fields" to "nextPageToken,files(id,name,mimeType)")
                    add("pageSize" to "1000")
                    add("q" to query)
                    pageToken?.let { add("pageToken" to it) }
                }

            val response =
                driveRequest(
                    token = token,
                    method = "GET",
                    url = driveApiUrl(driveApiEndpoints.filesEndpoint, parameters),
                ).asJsonObject()

            val files = response.optJSONArray("files") ?: JSONArray()
            for (index in 0 until files.length()) {
                val file = files.getJSONObject(index)
                children.add(
                    DriveFileMetadata(
                        id = file.getString("id"),
                        name = file.getString("name"),
                        mimeType = file.optString("mimeType"),
                    ),
                )
            }

            pageToken = response.optString("nextPageToken").takeIf(String::isNotBlank)
        } while (pageToken != null)

        return children
    }

    private suspend fun findChildByName(
        token: String,
        parentId: String,
        fileName: String,
        foldersOnly: Boolean = false,
    ): DriveFileMetadata? {
        val matches =
            listChildren(token, parentId, foldersOnly)
                .filter { it.name == fileName && (foldersOnly || !it.isFolder) }
        if (matches.size > 1) {
            if (foldersOnly) {
                throw duplicateDriveFolderException(fileName)
            }

            throw duplicateDriveFileException(fileName)
        }

        return matches.firstOrNull()
    }

    private suspend fun <T> runDriveOperation(
        interactive: Boolean,
        onError: (Throwable) -> CloudStorageException,
        bindAccountOnSuccess: (T) -> Boolean = { false },
        block: suspend (token: String) -> T,
    ): T {
        val started = monotonicTimeMs()
        val firstAccess =
            try {
                verifiedDriveAccessToken(interactive)
            } catch (error: Throwable) {
                if (error is CancellationException) throw error
                logDriveWarning("failed to get google drive access token", error)
                throw onError(error)
            }

        try {
            return finishDriveOperation(
                started = started,
                access = firstAccess,
                result = block(firstAccess.token),
                bindAccountOnSuccess = bindAccountOnSuccess,
            )
        } catch (error: Throwable) {
            if (error is CancellationException) throw error
            if (error is DriveHttpException && error.statusCode == HttpURLConnection.HTTP_UNAUTHORIZED) {
                runCatching {
                    driveAuthorization.clearToken(firstAccess.token)
                }.onFailure { tokenError ->
                    logDriveWarning("failed to clear expired drive token", tokenError)
                }

                val retryAccess =
                    try {
                        verifiedDriveAccessToken(interactive)
                    } catch (retryTokenError: Throwable) {
                        if (retryTokenError is CancellationException) throw retryTokenError
                        logDriveWarning("failed to refresh google drive access token", retryTokenError)
                        throw onError(retryTokenError)
                    }

                try {
                    return finishDriveOperation(
                        started = started,
                        access = retryAccess,
                        result = block(retryAccess.token),
                        bindAccountOnSuccess = bindAccountOnSuccess,
                        retry = true,
                    )
                } catch (retryError: Throwable) {
                    if (retryError is CancellationException) throw retryError
                    clearRejectedAuthorizationToken(retryAccess.token, retryError)
                    throw onError(retryError)
                }
            }

            clearRejectedAuthorizationToken(firstAccess.token, error)
            throw onError(error)
        }
    }

    private suspend fun verifiedDriveAccessToken(interactive: Boolean): DriveAccessToken =
        verifiedDriveAccessToken(interactive, retryIdentityLookup = true)

    private suspend fun verifiedDriveAccessToken(
        interactive: Boolean,
        retryIdentityLookup: Boolean,
    ): DriveAccessToken {
        val unresolvedAccess = driveAuthorization.accessToken(interactive)
        val access =
            try {
                unresolvedAccess.withResolvedAccountIdentity()
            } catch (error: Throwable) {
                if (error is CancellationException) throw error
                val tokenWasCleared = clearFailedIdentityLookupToken(unresolvedAccess.token, error)
                if (tokenWasCleared && retryIdentityLookup) {
                    return verifiedDriveAccessToken(interactive, retryIdentityLookup = false)
                }

                throw error
            }
        if (access != unresolvedAccess) {
            driveAuthorization.updateCachedToken(access)
        }

        try {
            verifyDriveAccountBinding(accountBindingStore, access.account, bindIfMissing = false)
        } catch (error: DriveAccountBindingException) {
            runCatching {
                driveAuthorization.clearToken(access.token)
            }.onFailure { tokenError ->
                logDriveWarning("failed to clear mismatched drive token", tokenError)
            }

            throw error
        }

        return access
    }

    private suspend fun DriveAccessToken.withResolvedAccountIdentity(): DriveAccessToken {
        if (account?.normalizedEmail != null) {
            return this
        }

        val resolvedAccount = driveAccountIdentity(token)
        val mergedAccount = account?.withMissingFieldsFrom(resolvedAccount) ?: resolvedAccount

        return if (mergedAccount == account) {
            this
        } else {
            copy(account = mergedAccount)
        }
    }

    private suspend fun driveAccountIdentity(token: String): DriveAccountIdentity? {
        return driveAccountIdentityFromAboutResponse(
            driveRequest(
                token = token,
                method = "GET",
                url = driveApiUrl(
                    driveApiEndpoints.aboutEndpoint,
                    listOf("fields" to "user(emailAddress)"),
                ),
            ).asJsonObject(),
        )
    }

    private suspend fun clearFailedIdentityLookupToken(token: String, error: Throwable): Boolean {
        val logMessage =
            when {
                error is DriveHttpException &&
                    error.statusCode == HttpURLConnection.HTTP_UNAUTHORIZED ->
                    "failed to clear expired drive token"
                error is DriveHttpException &&
                    error.statusCode == HttpURLConnection.HTTP_FORBIDDEN &&
                    error.isAuthorizationRejected() ->
                    "failed to clear rejected drive token"
                else -> return false
            }

        runCatching {
            driveAuthorization.clearToken(token)
        }.onFailure { tokenError ->
            logDriveWarning(logMessage, tokenError)
        }

        return true
    }

    private suspend fun <T> finishDriveOperation(
        started: Long,
        access: DriveAccessToken,
        result: T,
        bindAccountOnSuccess: (T) -> Boolean,
        retry: Boolean = false,
    ): T {
        if (bindAccountOnSuccess(result)) {
            bindDriveAccountAfterSuccessfulOperation(access)
        }

        val retryLabel = if (retry) " retry" else ""
        logDriveDebug("drive operation$retryLabel elapsed_ms=${monotonicTimeMs() - started}")
        return result
    }

    private suspend fun bindDriveAccountAfterSuccessfulOperation(access: DriveAccessToken) {
        try {
            verifyDriveAccountBinding(accountBindingStore, access.account)
        } catch (error: DriveAccountBindingException) {
            runCatching {
                driveAuthorization.clearToken(access.token)
            }.onFailure { tokenError ->
                logDriveWarning("failed to clear mismatched drive token", tokenError)
            }

            throw error
        }
    }

    private suspend fun clearRejectedAuthorizationToken(token: String, error: Throwable) {
        if (
            error is DriveHttpException &&
                error.statusCode == HttpURLConnection.HTTP_FORBIDDEN &&
                error.isAuthorizationRejected()
        ) {
            runCatching {
                driveAuthorization.clearToken(token)
            }.onFailure { tokenError ->
                logDriveWarning("failed to clear rejected drive token", tokenError)
            }
        }
    }

    private suspend fun driveRequest(
        token: String,
        method: String,
        url: String,
        body: ByteArray? = null,
        contentType: String? = null,
    ): DriveResponse =
        withContext(Dispatchers.IO) {
            val connection = (URL(url).openConnection() as HttpURLConnection)
            connection.requestMethod = method
            connection.connectTimeout = NETWORK_TIMEOUT_MS
            connection.readTimeout = NETWORK_TIMEOUT_MS
            connection.setRequestProperty("Authorization", "Bearer $token")
            connection.setRequestProperty("Accept", "application/json")

            if (body != null) {
                connection.doOutput = true
                connection.setRequestProperty("Content-Type", contentType)
                connection.outputStream.use { output ->
                    output.write(body)
                }
            }

            val statusCode = connection.responseCode
            val stream =
                if (statusCode in 200..299) {
                    connection.inputStream
                } else {
                    connection.errorStream ?: connection.inputStream
                }

            val responseBody = stream?.use { input -> input.readBytes() } ?: ByteArray(0)

            if (statusCode !in 200..299) {
                val responseText = responseBody.toString(Charsets.UTF_8)
                logDriveWarning("google drive request failed method=$method status=$statusCode")
                throw DriveHttpException(statusCode, responseText)
            }

            DriveResponse(statusCode, responseBody)
        }

    private fun DriveResponse.asJsonObject(): JSONObject =
        if (body.isEmpty()) {
            JSONObject()
        } else {
            JSONObject(body.toString(Charsets.UTF_8))
        }

    private data class DriveResponse(
        val statusCode: Int,
        val body: ByteArray,
    )

    private data class DriveFileMetadata(
        val id: String,
        val name: String,
        val mimeType: String,
    ) {
        val isFolder: Boolean
            get() = mimeType == DriveApi.FOLDER_MIME_TYPE
    }

    private fun List<DriveFileMetadata>.singleFolderChild(folderName: String): DriveFileMetadata? {
        val matches = filter { it.isFolder && it.name == folderName }
        if (matches.size > 1) {
            throw duplicateDriveFolderException(folderName)
        }

        return matches.firstOrNull()
    }

    private fun List<DriveFileMetadata>.backupFileLocations(
        locationForFileName: (String) -> String,
    ): List<String> =
        driveBackupFileLocations(
            fileNames = filterNot { it.isFolder }.map { it.name },
            locationForFileName = locationForFileName,
        )

    private data class DriveDeleteFailure(
        val fileId: String,
        val error: Throwable,
    )

    private fun aggregateDeleteFailures(failures: List<DriveDeleteFailure>): DriveHttpException {
        val statusCode =
            failures
                .mapNotNull { (it.error as? DriveHttpException)?.statusCode }
                .distinct()
                .singleOrNull()
                ?: HttpURLConnection.HTTP_INTERNAL_ERROR
        val body =
            failures.joinToString(
                separator = "; ",
                prefix = "failed to delete drive files: ",
            ) { failure ->
                deleteFailureDetail(failure.error)
            }

        return DriveHttpException(statusCode, body).apply {
            failures.forEach { addSuppressed(it.error) }
        }
    }

    private fun deleteFailureDetail(error: Throwable): String =
        when (error) {
            is DriveHttpException ->
                "status=${error.statusCode}"
            else ->
                "${error::class.java.simpleName}: ${error.message ?: "no message"}"
        }

    private object DriveApi {
        const val FOLDER_MIME_TYPE = "application/vnd.google-apps.folder"
    }

    companion object {
        private const val NETWORK_TIMEOUT_MS = 30_000
        private const val APP_DATA_FOLDER = "appDataFolder"
        private const val APP_DATA_SPACE = "appDataFolder"
    }
}

private const val DRIVE_ABOUT_ENDPOINT = "https://www.googleapis.com/drive/v3/about"
private const val DRIVE_FILES_ENDPOINT = "https://www.googleapis.com/drive/v3/files"
private const val DRIVE_UPLOAD_ENDPOINT = "https://www.googleapis.com/upload/drive/v3/files"
private const val HTTP_TOO_MANY_REQUESTS = 429
