package org.bitcoinppl.cove.cloudbackup

import android.content.Context
import java.net.HttpURLConnection
import kotlinx.coroutines.CancellationException
import org.bitcoinppl.cove_core.device.CloudAccessPolicy
import org.bitcoinppl.cove_core.device.CloudStorageAccess
import org.bitcoinppl.cove_core.device.CloudStorageException
import org.bitcoinppl.cove_core.device.CloudStorageInventorySnapshot
import org.bitcoinppl.cove_core.device.CloudSyncHealth
import org.bitcoinppl.cove_core.device.RemoteBackupLocation
import org.bitcoinppl.cove_core.device.cloudBackupLocationsSyncHealth

private const val GOOGLE_DRIVE_AUTHORIZATION_REQUIRED_MESSAGE =
    "Cove couldn't access Google Drive. Reconnect Google Drive, then try again."
private const val GOOGLE_DRIVE_OFFLINE_MESSAGE =
    "Cove couldn't reach Google Drive. Reconnect to the internet, then try again."
private const val GOOGLE_DRIVE_SYNC_FAILED_MESSAGE =
    "Cove couldn't check Google Drive sync. Please try again."

class AndroidCloudStorageAccess internal constructor(
    driveAuthorization: DriveAuthorization,
    private val accountBindingStore: DriveAccountBindingStore,
    driveApiEndpoints: DriveApiEndpoints = DriveApiEndpoints(),
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
            cacheKey = { driveAccountTokenCacheKey(accountBindingStore) },
        ),
        accountBindingStore,
    )

    private val httpClient = DriveHttpClient(driveApiEndpoints)
    private val folderResolver = DriveFolderResolver(httpClient, drivePathNamesProvider)
    private val tokenProvider = DriveTokenProvider(driveAuthorization, accountBindingStore, httpClient)

    internal suspend fun selectAccountForCloudBackup(transitionId: ULong): DriveAccountSelectionOutcome {
        val previouslySelectedIdentity = accountBindingStore.selectedIdentity()
        val access = tokenProvider.selectAccount()
        val identity = checkNotNull(access.account)

        if (previouslySelectedIdentity?.matches(identity) == true) {
            val enriched = previouslySelectedIdentity.verifiedMerge(identity)
            if (enriched != previouslySelectedIdentity) {
                accountBindingStore.bindIdentity(enriched)
            }

            logDriveDebug("selected google drive account is already bound to Cloud Backup")
            return DriveAccountSelectionOutcome.Unchanged
        }

        if (!accountBindingStore.stageIdentity(transitionId, identity)) {
            if (!accountBindingStore.rollbackStagedIdentity(transitionId)) {
                logDriveWarning("failed to clear unstaged drive account")
            }
            runCatching {
                tokenProvider.clearToken(access.token)
            }.onFailure { error ->
                logDriveWarning("failed to clear unstaged drive token", error)
            }

            error("google drive account selection could not be saved")
        }

        logDriveDebug("staged google drive account for Cloud Backup")
        return DriveAccountSelectionOutcome.Changed
    }

    internal fun pendingAccountSwitchTransitionId(): ULong? =
        accountBindingStore.pendingTransitionId()?.let { transitionId ->
            if (accountBindingStore.isIdentityStaged(transitionId)) {
                transitionId
            } else {
                check(accountBindingStore.rollbackStagedIdentity(transitionId)) {
                    "incomplete google drive account selection could not be cleared"
                }
                null
            }
        }

    internal fun commitAccountSwitch(transitionId: ULong): Boolean =
        accountBindingStore.commitStagedIdentity(transitionId)

    internal fun rollbackAccountSwitch(transitionId: ULong): Boolean =
        accountBindingStore.rollbackStagedIdentity(transitionId)

    private fun CloudAccessPolicy.allowsConsent(): Boolean =
        this == CloudAccessPolicy.CONSENT_ALLOWED

    override suspend fun uploadMasterKeyBackup(
        namespace: String,
        location: RemoteBackupLocation,
        data: ByteArray,
        policy: CloudAccessPolicy,
    ) {
        tokenProvider.runDriveOperation(
            interactive = policy.allowsConsent(),
            onError = { error ->
                mapDriveUploadError(error, location.errorId("master key backup"))
            },
            bindAccountOnSuccess = { true },
        ) { token ->
            val namespaceFolderId = folderResolver.ensureNamespaceFolderId(token, namespace)
            val parentId = folderResolver.ensureLocationParentFolderId(token, namespaceFolderId, location)
            folderResolver.upsertFile(
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
        tokenProvider.runDriveOperation(
            interactive = policy.allowsConsent(),
            onError = { error -> mapDriveUploadError(error, location.errorId(recordId)) },
            bindAccountOnSuccess = { true },
        ) { token ->
            val namespaceFolderId = folderResolver.ensureNamespaceFolderId(token, namespace)
            val parentId = folderResolver.ensureLocationParentFolderId(token, namespaceFolderId, location)
            folderResolver.upsertFile(
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
        tokenProvider.runDriveOperation(
            interactive = policy.allowsConsent(),
            onError = { error ->
                val errorId = locations.firstOrNull()?.errorId("master key backup") ?: "master key backup"
                mapDriveDownloadError(error, errorId)
            },
        ) { token ->
            val namespaceFolderId = folderResolver.requireNamespaceFolderId(token, namespace)
            val fileId =
                folderResolver.findFileAtLocations(
                    token = token,
                    namespaceFolderId = namespaceFolderId,
                    locations = locations,
                )?.id ?: throw DriveHttpException(404, "master key backup not found")
            httpClient.downloadFile(token, fileId)
        }

    override suspend fun downloadWalletBackup(
        namespace: String,
        recordId: String,
        locations: List<RemoteBackupLocation>,
        policy: CloudAccessPolicy,
    ): ByteArray =
        tokenProvider.runDriveOperation(
            interactive = policy.allowsConsent(),
            onError = { error ->
                val errorId = locations.firstOrNull()?.errorId(recordId) ?: recordId
                mapDriveDownloadError(error, errorId)
            },
            bindAccountOnSuccess = { true },
        ) { token ->
            val namespaceFolderId = folderResolver.requireNamespaceFolderId(token, namespace)
            val fileId =
                folderResolver.findFileAtLocations(
                    token = token,
                    namespaceFolderId = namespaceFolderId,
                    locations = locations,
                )?.id ?: throw DriveHttpException(404, "wallet backup not found")
            httpClient.downloadFile(token, fileId)
        }

    override suspend fun deleteWalletBackup(
        namespace: String,
        recordId: String,
        locations: List<RemoteBackupLocation>,
        policy: CloudAccessPolicy,
    ) {
        tokenProvider.runDriveOperation(
            interactive = policy.allowsConsent(),
            onError = { error ->
                val errorId = locations.firstOrNull()?.errorId(recordId) ?: recordId
                mapDriveDeleteError(error, errorId)
            },
        ) { token ->
            val namespaceFolderId = folderResolver.requireNamespaceFolderId(token, namespace)
            val files =
                folderResolver.findFilesAtLocations(
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
                    httpClient.driveRequest(
                        token = token,
                        method = "DELETE",
                        url = "${httpClient.endpoints.filesEndpoint}/${file.id}",
                    )
                } catch (error: Throwable) {
                    if (error is CancellationException) throw error
                    logDriveWarning("failed to delete drive backup file", error)
                    failures.add(DriveDeleteFailure(fileId = file.id, error = error))
                }
            }

            // report partial failures after best-effort cleanup so callers can retry remaining files
            if (failures.isNotEmpty()) {
                throw aggregateDeleteFailures(failures)
            }
        }
    }

    override suspend fun deleteNamespace(
        namespace: String,
        policy: CloudAccessPolicy,
    ) {
        tokenProvider.runDriveOperation(
            interactive = policy.allowsConsent(),
            onError = { error -> mapDriveDeleteError(error, namespace) },
        ) { token ->
            folderResolver.deleteNamespace(token, namespace)
        }
    }

    override suspend fun listNamespaces(policy: CloudAccessPolicy): List<String> =
        listNamespaces(interactive = policy.allowsConsent())

    override suspend fun listWalletFiles(
        namespace: String,
        policy: CloudAccessPolicy,
    ): List<String> =
        listWalletFiles(namespace, interactive = policy.allowsConsent())

    override suspend fun listWalletFilesSnapshot(
        namespace: String,
        policy: CloudAccessPolicy,
    ): CloudStorageInventorySnapshot =
        CloudStorageInventorySnapshot(
            names = listWalletFiles(namespace, interactive = policy.allowsConsent()),
            isComplete = true,
        )

    override suspend fun isBackupUploaded(
        namespace: String,
        recordId: String,
        locations: List<RemoteBackupLocation>,
        policy: CloudAccessPolicy,
    ): Boolean =
        tokenProvider.runDriveOperation(
            interactive = policy.allowsConsent(),
            onError = { error -> mapDriveListError(error) },
            bindAccountOnSuccess = { uploaded -> uploaded },
        ) { token ->
            val namespaceFolderId = folderResolver.findNamespaceFolderId(token, namespace)
                ?: return@runDriveOperation false
            folderResolver.findFileAtLocations(token, namespaceFolderId, locations) != null
        }

    override suspend fun overallSyncHealth(policy: CloudAccessPolicy): CloudSyncHealth =
        try {
            tokenProvider.runDriveOperation(
                interactive = policy.allowsConsent(),
                onError = { error -> throw error },
            ) { token ->
                cloudBackupLocationsSyncHealth(folderResolver.listNamespaceBackupLocations(token))
            }
        } catch (error: Throwable) {
            if (error is CancellationException) throw error
            val mapped = mapDriveListError(error)
            when (mapped) {
                is CloudStorageException.AuthorizationRequired ->
                    CloudSyncHealth.AuthorizationRequired(GOOGLE_DRIVE_AUTHORIZATION_REQUIRED_MESSAGE)
                is CloudStorageException.Offline -> CloudSyncHealth.Failed(GOOGLE_DRIVE_OFFLINE_MESSAGE)
                is CloudStorageException.NotAvailable -> CloudSyncHealth.Unavailable
                else -> CloudSyncHealth.Failed(GOOGLE_DRIVE_SYNC_FAILED_MESSAGE)
            }
        }

    private suspend fun listWalletFiles(
        namespace: String,
        interactive: Boolean,
    ): List<String> =
        tokenProvider.runDriveOperation(
            interactive = interactive,
            onError = { error -> mapDriveListError(error) },
            bindAccountOnSuccess = { true },
        ) { token ->
            folderResolver.listWalletFiles(token, namespace)
        }

    private suspend fun listNamespaces(
        interactive: Boolean,
    ): List<String> =
        tokenProvider.runDriveOperation(
            interactive = interactive,
            onError = { error -> mapDriveListError(error) },
        ) { token ->
            folderResolver.listNamespaces(token)
        }

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
}

internal const val DRIVE_ABOUT_ENDPOINT = "https://www.googleapis.com/drive/v3/about"
internal const val DRIVE_FILES_ENDPOINT = "https://www.googleapis.com/drive/v3/files"
internal const val DRIVE_UPLOAD_ENDPOINT = "https://www.googleapis.com/upload/drive/v3/files"
internal const val HTTP_TOO_MANY_REQUESTS = 429
