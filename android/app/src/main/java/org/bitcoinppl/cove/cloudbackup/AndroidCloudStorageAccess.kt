package org.bitcoinppl.cove.cloudbackup

import android.content.Context
import android.net.Uri
import com.google.android.gms.common.api.ApiException
import com.google.android.gms.common.api.CommonStatusCodes
import java.io.ByteArrayOutputStream
import java.io.IOException
import java.net.HttpURLConnection
import java.net.SocketTimeoutException
import java.net.URL
import java.net.UnknownHostException
import java.util.concurrent.ConcurrentHashMap
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.coroutines.withContext
import org.bitcoinppl.cove.Log
import org.bitcoinppl.cove_core.csppMasterKeyRecordId
import org.bitcoinppl.cove_core.device.CloudAccessPolicy
import org.bitcoinppl.cove_core.device.CloudStorageAccess
import org.bitcoinppl.cove_core.device.CloudStorageException
import org.bitcoinppl.cove_core.device.CloudSyncHealth
import org.json.JSONArray
import org.json.JSONObject

internal fun syncHealthForNamespaceFiles(
    namespaceFiles: List<List<String>>,
    hasUploadedBackupFiles: (List<String>) -> Boolean,
    hasMasterKeyBackup: (List<String>) -> Boolean,
): CloudSyncHealth =
    when {
        namespaceFiles.none(hasUploadedBackupFiles) -> CloudSyncHealth.NoFiles
        namespaceFiles.all(hasMasterKeyBackup) -> CloudSyncHealth.AllUploaded
        else -> CloudSyncHealth.Failed("cloud backup is incomplete")
    }

internal fun hasUploadedBackupFiles(fileNames: List<String>): Boolean =
    hasUploadedBackupFiles(
        fileNames = fileNames,
        masterKeyFileName = DrivePaths.masterKeyFileName,
        isWalletFile = DrivePaths::isWalletFile,
    )

internal fun hasUploadedBackupFiles(
    fileNames: List<String>,
    masterKeyFileName: String,
    isWalletFile: (String) -> Boolean,
): Boolean =
    fileNames.any { it == masterKeyFileName || isWalletFile(it) }

internal fun hasMasterKeyBackup(fileNames: List<String>): Boolean =
    hasMasterKeyBackup(
        fileNames = fileNames,
        masterKeyFileName = DrivePaths.masterKeyFileName,
    )

internal fun hasMasterKeyBackup(
    fileNames: List<String>,
    masterKeyFileName: String,
): Boolean =
    fileNames.contains(masterKeyFileName)

internal fun driveFileNameForRecordId(recordId: String): String =
    driveFileNameForRecordId(
        recordId = recordId,
        masterKeyRecordId = csppMasterKeyRecordId(),
        masterKeyFileName = { DrivePaths.masterKeyFileName },
        walletFileName = DrivePaths::walletFileName,
    )

internal fun driveFileNameForRecordId(
    recordId: String,
    masterKeyRecordId: String,
    masterKeyFileName: () -> String,
    walletFileName: (String) -> String,
): String =
    if (recordId == masterKeyRecordId) {
        masterKeyFileName()
    } else {
        walletFileName(recordId)
    }

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

internal fun driveQuotaReasons(body: String): Set<DriveQuotaReason> {
    val root = runCatching { JSONObject(body) }.getOrNull() ?: return emptySet()
    val error = root.optJSONObject("error") ?: return emptySet()
    val reasons = mutableSetOf<DriveQuotaReason>()

    DriveQuotaReason.from(error.optString("reason"))?.let(reasons::add)

    val errors = error.optJSONArray("errors")
    if (errors != null) {
        for (index in 0 until errors.length()) {
            val reason = errors.optJSONObject(index)?.optString("reason") ?: continue
            DriveQuotaReason.from(reason)?.let(reasons::add)
        }
    }

    return reasons
}

private fun DriveHttpException.isQuotaExceeded(): Boolean =
    driveQuotaReasons(body).isNotEmpty()

internal fun mapDriveUploadError(error: Throwable, target: String): CloudStorageException =
    when (error) {
        is AuthorizationRequiredException ->
            CloudStorageException.AuthorizationRequired("google drive authorization is required")
        is ApiException ->
            if (error.statusCode == CommonStatusCodes.CANCELED) {
                CloudStorageException.AuthorizationRequired("google drive authorization was cancelled")
            } else {
                CloudStorageException.NotAvailable(error.message ?: "google drive is unavailable")
            }
        is DriveHttpException ->
            when (error.statusCode) {
                HttpURLConnection.HTTP_UNAUTHORIZED ->
                    CloudStorageException.AuthorizationRequired("google drive authorization is required")
                HttpURLConnection.HTTP_NOT_FOUND -> CloudStorageException.NotFound(target)
                HTTP_TOO_MANY_REQUESTS -> CloudStorageException.QuotaExceeded()
                HttpURLConnection.HTTP_FORBIDDEN ->
                    if (error.isQuotaExceeded()) {
                        CloudStorageException.QuotaExceeded()
                    } else {
                        CloudStorageException.AuthorizationRequired("google drive access was rejected")
                    }
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
        is AuthorizationRequiredException ->
            CloudStorageException.AuthorizationRequired("google drive authorization is required")
        is ApiException ->
            if (error.statusCode == CommonStatusCodes.CANCELED) {
                CloudStorageException.AuthorizationRequired("google drive authorization was cancelled")
            } else {
                CloudStorageException.NotAvailable(error.message ?: "google drive is unavailable")
            }
        is DriveHttpException ->
            when (error.statusCode) {
                HttpURLConnection.HTTP_UNAUTHORIZED ->
                    CloudStorageException.AuthorizationRequired("google drive authorization is required")
                HTTP_TOO_MANY_REQUESTS -> CloudStorageException.QuotaExceeded()
                HttpURLConnection.HTTP_NOT_FOUND -> CloudStorageException.NotFound("drive file")
                HttpURLConnection.HTTP_FORBIDDEN ->
                    if (error.isQuotaExceeded()) {
                        CloudStorageException.QuotaExceeded()
                    } else {
                        CloudStorageException.AuthorizationRequired("google drive access was rejected")
                    }
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

class AndroidCloudStorageAccess internal constructor(
    private val driveAuthorization: DriveAuthorization,
) : CloudStorageAccess {
    constructor(context: Context) : this(DriveAuthorizationHelper(context))

    private val namespacesRootFolderMutex = Mutex()
    private val namespaceFolderMutexes = ConcurrentHashMap<String, Mutex>()

    private fun CloudAccessPolicy.allowsConsent(): Boolean =
        this == CloudAccessPolicy.CONSENT_ALLOWED

    override suspend fun uploadMasterKeyBackup(
        namespace: String,
        data: ByteArray,
        policy: CloudAccessPolicy,
    ) {
        runDriveOperation(
            interactive = policy.allowsConsent(),
            onError = { error -> mapDriveUploadError(error, DrivePaths.masterKeyFileName) },
        ) { token ->
            val namespaceFolderId = ensureNamespaceFolderId(token, namespace)
            upsertFile(
                token = token,
                parentId = namespaceFolderId,
                fileName = DrivePaths.masterKeyFileName,
                data = data,
            )
        }
    }

    override suspend fun uploadWalletBackup(
        namespace: String,
        recordId: String,
        data: ByteArray,
        policy: CloudAccessPolicy,
    ) {
        runDriveOperation(
            interactive = policy.allowsConsent(),
            onError = { error -> mapDriveUploadError(error, recordId) },
        ) { token ->
            val namespaceFolderId = ensureNamespaceFolderId(token, namespace)
            upsertFile(
                token = token,
                parentId = namespaceFolderId,
                fileName = DrivePaths.walletFileName(recordId),
                data = data,
            )
        }
    }

    override suspend fun downloadMasterKeyBackup(
        namespace: String,
        policy: CloudAccessPolicy,
    ): ByteArray =
        runDriveOperation(
            interactive = policy.allowsConsent(),
            onError = { error -> mapDriveDownloadError(error, DrivePaths.masterKeyFileName) },
        ) { token ->
            val namespaceFolderId = requireNamespaceFolderId(token, namespace)
            val fileId =
                findChildByName(
                    token = token,
                    parentId = namespaceFolderId,
                    fileName = DrivePaths.masterKeyFileName,
                )?.id ?: throw DriveHttpException(404, "master key backup not found")
            downloadFile(token, fileId)
        }

    override suspend fun downloadWalletBackup(
        namespace: String,
        recordId: String,
        policy: CloudAccessPolicy,
    ): ByteArray =
        runDriveOperation(
            interactive = policy.allowsConsent(),
            onError = { error -> mapDriveDownloadError(error, recordId) },
        ) { token ->
            val namespaceFolderId = requireNamespaceFolderId(token, namespace)
            val fileId =
                findChildByName(
                    token = token,
                    parentId = namespaceFolderId,
                    fileName = DrivePaths.walletFileName(recordId),
                )?.id ?: throw DriveHttpException(404, "wallet backup not found")
            downloadFile(token, fileId)
        }

    override suspend fun deleteWalletBackup(
        namespace: String,
        recordId: String,
        policy: CloudAccessPolicy,
    ) {
        runDriveOperation(
            interactive = policy.allowsConsent(),
            onError = { error -> mapDriveDeleteError(error, recordId) },
        ) { token ->
            val namespaceFolderId = requireNamespaceFolderId(token, namespace)
            val file =
                findChildByName(
                    token = token,
                    parentId = namespaceFolderId,
                    fileName = DrivePaths.walletFileName(recordId),
                ) ?: throw DriveHttpException(404, "wallet backup not found")

            driveRequest(
                token = token,
                method = "DELETE",
                url = "${DriveApi.FILES_ENDPOINT}/${file.id}",
            )
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
        policy: CloudAccessPolicy,
    ): Boolean =
        runDriveOperation(
            interactive = policy.allowsConsent(),
            onError = { error -> mapDriveListError(error) },
        ) { token ->
            val namespaceFolderId = findNamespaceFolderId(token, namespace) ?: return@runDriveOperation false
            findChildByName(
                token = token,
                parentId = namespaceFolderId,
                fileName = driveFileNameForRecordId(recordId),
            ) != null
        }

    override suspend fun overallSyncHealth(policy: CloudAccessPolicy): CloudSyncHealth =
        try {
            runDriveOperation(
                interactive = policy.allowsConsent(),
                onError = { error -> throw error },
            ) { token ->
                val namespacesRootId = findNamespacesRootFolderId(token) ?: return@runDriveOperation CloudSyncHealth.NoFiles
                val namespaces =
                    listChildren(
                        token = token,
                        parentId = namespacesRootId,
                        foldersOnly = true,
                    )
                if (namespaces.isEmpty()) {
                    return@runDriveOperation CloudSyncHealth.NoFiles
                }

                val namespaceFiles =
                    namespaces.map { namespace ->
                        listChildren(
                            token = token,
                            parentId = namespace.id,
                            foldersOnly = false,
                        ).map { it.name }
                    }

                syncHealthForNamespaceFiles(
                    namespaceFiles = namespaceFiles,
                    hasUploadedBackupFiles = ::hasUploadedBackupFiles,
                    hasMasterKeyBackup = ::hasMasterKeyBackup,
                )
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
            fileName = DrivePaths.namespacesRootFolderName,
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
                            folderName = DrivePaths.namespacesRootFolderName,
                        )
                    findNamespacesRootFolderId(token) ?: createdId
                }
        }

    private suspend fun ensureNamespaceFolderId(
        token: String,
        namespace: String,
    ): String {
        val rootId = ensureNamespacesRootFolderId(token)
        val mutex = namespaceFolderMutexes.getOrPut(namespace) { Mutex() }
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
        val rootId = findNamespacesRootFolderId(token) ?: throw DriveHttpException(404, "namespaces root not found")
        return findChildByName(
            token = token,
            parentId = rootId,
            fileName = namespace,
            foldersOnly = true,
        )?.id ?: throw DriveHttpException(404, "namespace not found")
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
                url = DriveApi.FILES_ENDPOINT,
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
                "${DriveApi.UPLOAD_ENDPOINT}?uploadType=multipart"
            } else {
                "${DriveApi.UPLOAD_ENDPOINT}/${existing.id}?uploadType=multipart"
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
        ) { token ->
            val namespaceFolderId = requireNamespaceFolderId(token, namespace)
            listChildren(
                token = token,
                parentId = namespaceFolderId,
                foldersOnly = false,
            ).map { it.name }
                .filter(DrivePaths::isWalletFile)
        }

    private suspend fun listNamespaces(
        interactive: Boolean,
    ): List<String> =
        runDriveOperation(
            interactive = interactive,
            onError = { error -> mapDriveListError(error) },
        ) { token ->
            val namespacesRootId = findNamespacesRootFolderId(token) ?: return@runDriveOperation emptyList()
            listChildren(
                token = token,
                parentId = namespacesRootId,
                foldersOnly = true,
            ).map { it.name }
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
            url = "${DriveApi.FILES_ENDPOINT}/$fileId?alt=media",
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
            val builder =
                Uri
                    .parse(DriveApi.FILES_ENDPOINT)
                    .buildUpon()
                    .appendQueryParameter("spaces", APP_DATA_SPACE)
                    .appendQueryParameter("fields", "nextPageToken,files(id,name,mimeType)")
                    .appendQueryParameter("pageSize", "1000")
                    .appendQueryParameter("q", query)

            pageToken?.let { builder.appendQueryParameter("pageToken", it) }

            val response =
                driveRequest(
                    token = token,
                    method = "GET",
                    url = builder.build().toString(),
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
    ): DriveFileMetadata? =
        listChildren(token, parentId, foldersOnly).firstOrNull { it.name == fileName }

    private suspend fun <T> runDriveOperation(
        interactive: Boolean,
        onError: (Throwable) -> CloudStorageException,
        block: suspend (token: String) -> T,
    ): T {
        val firstToken =
            try {
                driveAuthorization.accessToken(interactive)
            } catch (error: Throwable) {
                if (error is CancellationException) throw error
                throw onError(error)
            }

        try {
            return block(firstToken)
        } catch (error: Throwable) {
            if (error is CancellationException) throw error
            if (error is DriveHttpException && error.statusCode == HttpURLConnection.HTTP_UNAUTHORIZED) {
                runCatching {
                    driveAuthorization.clearToken(firstToken)
                }.onFailure { tokenError ->
                    Log.w("AndroidCloudStorage", "failed to clear expired drive token", tokenError)
                }

                val retryToken =
                    try {
                        driveAuthorization.accessToken(interactive)
                    } catch (retryTokenError: Throwable) {
                        if (retryTokenError is CancellationException) throw retryTokenError
                        throw onError(retryTokenError)
                    }

                try {
                    return block(retryToken)
                } catch (retryError: Throwable) {
                    if (retryError is CancellationException) throw retryError
                    throw onError(retryError)
                }
            }

            throw onError(error)
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
                throw DriveHttpException(statusCode, responseBody.toString(Charsets.UTF_8))
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
    )

    private object DriveApi {
        const val FILES_ENDPOINT = "https://www.googleapis.com/drive/v3/files"
        const val UPLOAD_ENDPOINT = "https://www.googleapis.com/upload/drive/v3/files"
        const val FOLDER_MIME_TYPE = "application/vnd.google-apps.folder"
    }

    companion object {
        private const val NETWORK_TIMEOUT_MS = 30_000
        private const val APP_DATA_FOLDER = "appDataFolder"
        private const val APP_DATA_SPACE = "appDataFolder"
    }
}

private const val HTTP_TOO_MANY_REQUESTS = 429
