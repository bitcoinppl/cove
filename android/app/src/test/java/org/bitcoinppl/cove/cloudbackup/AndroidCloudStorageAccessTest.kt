package org.bitcoinppl.cove.cloudbackup

import java.io.IOException
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeout
import org.bitcoinppl.cove_core.device.CloudAccessPolicy
import org.bitcoinppl.cove_core.DriveAccountSwitchPlatformState
import org.bitcoinppl.cove_core.device.CloudStorageException
import org.bitcoinppl.cove_core.device.CloudSyncHealth
import org.bitcoinppl.cove_core.device.RemoteBackupLocation
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class AndroidCloudStorageAccessTest {
    @Test
    fun explicitDriveAccountSelectionStagesReplacementUntilCommit() =
        runBlocking {
            val original = DriveAccountIdentity(googleAccountId = "account-1", email = "person@example.com")
            val replacement = DriveAccountIdentity(googleAccountId = "account-2", email = "other@example.com")
            val store = TestDriveAccountBindingStore(original)
            val authorization = RecordingDriveAuthorization().apply { account = replacement }
            val storage = AndroidCloudStorageAccess(authorization, store)

            val outcome = storage.selectAccountForCloudBackup(1UL)

            assertEquals(DriveAccountSelectionOutcome.Changed, outcome)
            assertEquals(replacement, store.selectedIdentity())
            assertEquals(original, store.committedIdentity())
            assertEquals(
                DriveAccountBindingState.Staged(1UL, original, replacement),
                store.state(),
            )
            assertEquals(
                DriveAccountSwitchPlatformState.Staged(1UL),
                storage.driveAccountSwitchPlatformState(),
            )
            assertEquals(DriveAccountTransitionResult.Applied, storage.commitAccountSwitch(1UL))
            assertEquals(replacement, store.committedIdentity())
            assertEquals(
                DriveAccountBindingState.Committed(1UL, replacement),
                store.state(),
            )
            assertEquals(
                DriveAccountSwitchPlatformState.Committed(1UL),
                storage.driveAccountSwitchPlatformState(),
            )
            assertEquals(listOf(true), authorization.accessRequests)
        }

    @Test
    fun selectingBoundDriveAccountDoesNotStageReinitialization() =
        runBlocking {
            val original = DriveAccountIdentity(googleAccountId = "account-1", email = "person@example.com")
            val refreshed =
                DriveAccountIdentity(
                    googleAccountId = "account-1",
                    drivePermissionId = "permission-1",
                    email = "person@example.com",
                )
            val store = TestDriveAccountBindingStore(original)
            val authorization = RecordingDriveAuthorization().apply { account = refreshed }
            val storage = AndroidCloudStorageAccess(authorization, store)

            val outcome = storage.selectAccountForCloudBackup(1UL)

            assertEquals(DriveAccountSelectionOutcome.Unchanged, outcome)
            assertEquals(refreshed, store.committedIdentity())
            assertEquals(DriveAccountBindingState.Bound(refreshed), store.state())
        }

    @Test
    fun rolledBackDriveAccountSelectionRestoresCommittedBinding() =
        runBlocking {
            val original = DriveAccountIdentity(googleAccountId = "account-1", email = "person@example.com")
            val replacement = DriveAccountIdentity(googleAccountId = "account-2", email = "other@example.com")
            val store = TestDriveAccountBindingStore(original)
            val authorization = RecordingDriveAuthorization().apply { account = replacement }
            val storage = AndroidCloudStorageAccess(authorization, store)

            storage.selectAccountForCloudBackup(7UL)

            assertEquals(DriveAccountTransitionResult.Applied, storage.rollbackAccountSwitch(7UL))
            assertEquals(original, store.selectedIdentity())
            assertEquals(original, store.committedIdentity())
            assertEquals(DriveAccountBindingState.Bound(original), store.state())
        }

    @Test
    fun failedDriveAccountStagingPreservesBindingAndClearsSelectedToken() =
        runBlocking {
            val original = DriveAccountIdentity(googleAccountId = "account-1", email = "person@example.com")
            val replacement = DriveAccountIdentity(googleAccountId = "account-2", email = "other@example.com")
            val store = TestDriveAccountBindingStore(original).apply {
                stageSucceeds = false
                retainFailedStage = true
            }
            val authorization = RecordingDriveAuthorization().apply { account = replacement }
            val storage = AndroidCloudStorageAccess(authorization, store)

            val error = captureError { storage.selectAccountForCloudBackup(7UL) }

            assertTrue(error is IllegalStateException)
            assertEquals(original, store.selectedIdentity())
            assertEquals(original, store.committedIdentity())
            assertEquals(DriveAccountBindingState.Bound(original), store.state())
            assertEquals(listOf("token-1"), authorization.clearedTokens)
        }

    @Test
    fun scP05SilentAuthoritativeEmptySnapshotCompletesWithoutRetryOrConsent() =
        runBlocking {
            MockDriveServer().use { server ->
                val namespace = "0123456789abcdef0123456789abcdef"
                server.enqueue(
                    200,
                    driveFilesResponse("namespaces-root", testDrivePathNames.namespacesRootFolderName),
                )
                server.enqueue(200, driveFilesResponse("namespace-folder", namespace))
                server.enqueue(200, """{"files":[]}""")

                val authorization = RecordingDriveAuthorization()
                val storage = storageFor(server, authorization)
                val snapshot = withTimeout(1_000) {
                    storage.listWalletFilesSnapshot(namespace, CloudAccessPolicy.SILENT)
                }

                assertTrue(snapshot.isComplete)
                assertTrue(snapshot.names.isEmpty())
                assertEquals(listOf(false), authorization.accessRequests)

                val requests = server.requests(3)
                assertEquals(3, requests.size)
                assertTrue(requests.all { it.method == "GET" })
            }
        }

    @Test
    fun cloudDiscoveryOnlyAllowsInteractiveAuthorizationWithExplicitConsent() =
        runBlocking {
            val selected = DriveAccountIdentity(googleAccountId = "account-1", email = "person@example.com")
            val actual = DriveAccountIdentity(googleAccountId = "account-2", email = "other@example.com")
            val authorization = RecordingDriveAuthorization().apply {
                account = actual
            }
            val storage = AndroidCloudStorageAccess(
                authorization,
                TestDriveAccountBindingStore(selected),
            )

            captureError {
                storage.listNamespaces(CloudAccessPolicy.SILENT)
            }
            captureError {
                storage.listNamespaces(CloudAccessPolicy.CONSENT_ALLOWED)
            }

            assertEquals(listOf(false, true), authorization.accessRequests)
        }

    @Test
    fun wrongDriveAccountBlocksOperationsBeforeRemoteMutation() =
        runBlocking {
            val selected = DriveAccountIdentity(googleAccountId = "account-1", email = "person@example.com")
            val actual = DriveAccountIdentity(googleAccountId = "account-2", email = "other@example.com")
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
    fun overallSyncHealthDoesNotPublishDriveDiagnostics() =
        runBlocking {
            val authorizationRequired = AndroidCloudStorageAccess(
                FailingDriveAuthorization(DriveHttpException(401, "account=person@example.com")),
                TestDriveAccountBindingStore(),
            ).overallSyncHealth(CloudAccessPolicy.SILENT)
            val offline = AndroidCloudStorageAccess(
                FailingDriveAuthorization(IOException("network for secret account failed")),
                TestDriveAccountBindingStore(),
            ).overallSyncHealth(CloudAccessPolicy.SILENT)
            val failed = AndroidCloudStorageAccess(
                FailingDriveAuthorization(DriveHttpException(404, "record=secret")),
                TestDriveAccountBindingStore(),
            ).overallSyncHealth(CloudAccessPolicy.SILENT)

            assertEquals(
                CloudSyncHealth.AuthorizationRequired(
                    "Cove couldn't access Google Drive. Reconnect Google Drive, then try again.",
                ),
                authorizationRequired,
            )
            assertEquals(
                CloudSyncHealth.Failed(
                    "Cove couldn't reach Google Drive. Reconnect to the internet, then try again.",
                ),
                offline,
            )
            assertEquals(
                CloudSyncHealth.Failed("Cove couldn't check Google Drive sync. Please try again."),
                failed,
            )
        }

    private fun storageFor(
        server: MockDriveServer,
        authorization: DriveAuthorization,
    ): AndroidCloudStorageAccess =
        AndroidCloudStorageAccess(
            driveAuthorization = authorization,
            accountBindingStore = TestDriveAccountBindingStore(),
            driveApiEndpoints =
                DriveApiEndpoints(
                    aboutEndpoint = "${server.baseUrl}/about",
                    filesEndpoint = "${server.baseUrl}/files",
                    uploadEndpoint = "${server.baseUrl}/upload",
                ),
            drivePathNamesProvider = { testDrivePathNames },
        )

    private fun driveFilesResponse(
        id: String,
        name: String,
    ): String =
        """
        {
            "files": [
                {
                    "id": "$id",
                    "name": "$name",
                    "mimeType": "application/vnd.google-apps.folder"
                }
            ]
        }
        """.trimIndent()
}
