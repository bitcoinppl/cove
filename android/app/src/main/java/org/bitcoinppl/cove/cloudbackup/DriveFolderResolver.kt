package org.bitcoinppl.cove.cloudbackup

import java.net.HttpURLConnection
import java.util.concurrent.ConcurrentHashMap
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import org.bitcoinppl.cove_core.device.RemoteBackupLocation
import org.json.JSONArray
import org.json.JSONObject

internal class DriveFolderResolver(
    private val httpClient: DriveHttpClient,
    drivePathNamesProvider: () -> DrivePathNames = { DrivePaths.defaultNames },
) {
    private val tree = DriveFolderTree(httpClient, drivePathNamesProvider)
    private val fileLocator = DriveFileLocator(tree)
    private val backupLocationLister = DriveBackupLocationLister(tree)

    suspend fun ensureNamespaceFolderId(
        token: String,
        namespace: String,
    ): String =
        tree.ensureNamespaceFolderId(token, namespace)

    suspend fun findNamespaceFolderId(
        token: String,
        namespace: String,
    ): String? =
        tree.findNamespaceFolderId(token, namespace)

    suspend fun requireNamespaceFolderId(
        token: String,
        namespace: String,
    ): String =
        tree.requireNamespaceFolderId(token, namespace)

    suspend fun ensureLocationParentFolderId(
        token: String,
        namespaceFolderId: String,
        location: RemoteBackupLocation,
    ): String {
        var parentId = namespaceFolderId
        for (folderName in location.parts.parentFolders) {
            parentId =
                tree.ensureChildFolderId(
                    token = token,
                    parentId = parentId,
                    folderName = folderName,
                )
        }

        return parentId
    }

    suspend fun findFilesAtLocations(
        token: String,
        namespaceFolderId: String,
        locations: List<RemoteBackupLocation>,
    ): List<DriveFileMetadata> =
        fileLocator.findFilesAtLocations(token, namespaceFolderId, locations)

    suspend fun findFileAtLocations(
        token: String,
        namespaceFolderId: String,
        locations: List<RemoteBackupLocation>,
    ): DriveFileMetadata? =
        fileLocator.findFileAtLocations(token, namespaceFolderId, locations)

    suspend fun upsertFile(
        token: String,
        parentId: String,
        fileName: String,
        data: ByteArray,
    ) {
        val existing =
            tree.findChildByName(
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
        val body = httpClient.buildMultipartBody(boundary, metadata, data)
        val url =
            if (existing == null) {
                "${httpClient.endpoints.uploadEndpoint}?uploadType=multipart"
            } else {
                "${httpClient.endpoints.uploadEndpoint}/${existing.id}?uploadType=multipart"
            }

        httpClient.driveRequest(
            token = token,
            method = if (existing == null) "POST" else "PATCH",
            url = url,
            body = body,
            contentType = "multipart/related; boundary=$boundary",
        )
    }

    suspend fun listWalletFiles(
        token: String,
        namespace: String,
    ): List<String> =
        backupLocationLister.listWalletFiles(token, namespace)

    suspend fun listNamespaceBackupLocations(token: String): List<List<String>> =
        backupLocationLister.listNamespaceBackupLocations(token)

    suspend fun listNamespaces(token: String): List<String> =
        backupLocationLister.listNamespaces(token)

    suspend fun deleteNamespace(
        token: String,
        namespace: String,
    ) {
        tree.deleteNamespace(token, namespace)
    }
}

private class DriveFolderTree(
    private val httpClient: DriveHttpClient,
    drivePathNamesProvider: () -> DrivePathNames,
) {
    private val namespacesRootFolderMutex = Mutex()
    private val namespaceFolderMutexes = ConcurrentHashMap<String, Mutex>()
    private val childFolderMutexes = ConcurrentHashMap<String, Mutex>()
    val drivePathNames: DrivePathNames by lazy(drivePathNamesProvider)

    suspend fun ensureNamespaceFolderId(
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

    suspend fun findNamespaceFolderId(
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

    suspend fun requireNamespaceFolderId(
        token: String,
        namespace: String,
    ): String {
        val rootId =
            findNamespacesRootFolderId(token)
                ?: throw DriveHttpException(HttpURLConnection.HTTP_NOT_FOUND, "namespaces root not found")
        return findChildByName(
            token = token,
            parentId = rootId,
            fileName = namespace,
            foldersOnly = true,
        )?.id ?: throw DriveHttpException(HttpURLConnection.HTTP_NOT_FOUND, "namespace not found")
    }

    suspend fun ensureChildFolderId(
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

    suspend fun findNamespacesRootFolderId(token: String): String? =
        findChildByName(
            token = token,
            parentId = APP_DATA_FOLDER,
            fileName = drivePathNames.namespacesRootFolderName,
            foldersOnly = true,
        )?.id

    suspend fun listChildren(
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
                httpClient.driveRequest(
                    token = token,
                    method = "GET",
                    url = driveApiUrl(httpClient.endpoints.filesEndpoint, parameters),
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

    suspend fun findChildByName(
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

    suspend fun deleteNamespace(
        token: String,
        namespace: String,
    ) {
        val mutex = namespaceFolderMutexes.computeIfAbsent(namespace) { Mutex() }
        mutex.withLock {
            val namespaceFolderId =
                findNamespaceFolderId(token, namespace)
                    ?: throw DriveHttpException(HttpURLConnection.HTTP_NOT_FOUND, "namespace not found")

            httpClient.driveRequest(
                token = token,
                method = "DELETE",
                url = "${httpClient.endpoints.filesEndpoint}/$namespaceFolderId",
            )
        }
    }

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
            httpClient.driveRequest(
                token = token,
                method = "POST",
                url = httpClient.endpoints.filesEndpoint,
                body = metadata.toString().toByteArray(),
                contentType = "application/json; charset=utf-8",
            ).asJsonObject()

        return response.getString("id")
    }

    private companion object {
        const val APP_DATA_FOLDER = "appDataFolder"
        const val APP_DATA_SPACE = "appDataFolder"
    }
}

private class DriveFileLocator(
    private val tree: DriveFolderTree,
) {
    suspend fun findFilesAtLocations(
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

    suspend fun findFileAtLocations(
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
                tree.findChildByName(
                    token = token,
                    parentId = parentId,
                    fileName = folderName,
                    foldersOnly = true,
                )?.id ?: return null
        }

        return tree.findChildByName(
            token = token,
            parentId = parentId,
            fileName = parts.fileName,
        )
    }
}

private class DriveBackupLocationLister(
    private val tree: DriveFolderTree,
) {
    suspend fun listWalletFiles(
        token: String,
        namespace: String,
    ): List<String> {
        val namespaceFolderId = tree.requireNamespaceFolderId(token, namespace)
        return listBackupLocations(
            token = token,
            namespaceFolderId = namespaceFolderId,
        ).filter(tree.drivePathNames::isWalletFile)
    }

    suspend fun listNamespaceBackupLocations(token: String): List<List<String>> {
        val namespacesRootId = tree.findNamespacesRootFolderId(token) ?: return emptyList()

        return listNamespaceFolders(token, namespacesRootId)
            .filter { namespace -> isValidCloudBackupNamespaceId(namespace.name) }
            .map { namespace ->
                listBackupLocations(
                    token = token,
                    namespaceFolderId = namespace.id,
                )
            }
    }

    suspend fun listNamespaces(token: String): List<String> {
        val namespacesRootId = tree.findNamespacesRootFolderId(token) ?: return emptyList()
        return listNamespaceFolders(token, namespacesRootId).map { it.name }
    }

    private suspend fun listBackupLocations(
        token: String,
        namespaceFolderId: String,
    ): List<String> {
        val immediateChildren =
            tree.listChildren(
                token = token,
                parentId = namespaceFolderId,
                foldersOnly = false,
            )

        val locations =
            immediateChildren
                .backupFileLocations { it }
                .toMutableList()

        immediateChildren
            .singleFolderChild(tree.drivePathNames.masterKeyFolderName)
            ?.let { masterKeyFolder ->
                tree.listChildren(
                    token = token,
                    parentId = masterKeyFolder.id,
                    foldersOnly = false,
                ).backupFileLocations { "${tree.drivePathNames.masterKeyFolderName}/$it" }
                    .let(locations::addAll)
            }

        immediateChildren
            .singleFolderChild(tree.drivePathNames.walletsFolderName)
            ?.let { walletsFolder ->
                tree.listChildren(
                    token = token,
                    parentId = walletsFolder.id,
                    foldersOnly = false,
                ).backupFileLocations(tree.drivePathNames::walletLocationForFileName)
                    .let(locations::addAll)
            }

        return locations
    }

    private suspend fun listNamespaceFolders(
        token: String,
        namespacesRootId: String,
    ): List<DriveFileMetadata> {
        val namespaces =
            tree.listChildren(
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
}

internal data class DriveFileMetadata(
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

private object DriveApi {
    const val FOLDER_MIME_TYPE = "application/vnd.google-apps.folder"
}
