package org.bitcoinppl.cove.cloudbackup

import kotlinx.coroutines.runBlocking
import org.bitcoinppl.cove_core.device.CloudAccessPolicy
import org.bitcoinppl.cove_core.device.CloudStorageException
import org.bitcoinppl.cove_core.device.RemoteBackupLocation
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class AndroidCloudStorageAccessTest {
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
}
