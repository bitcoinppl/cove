package org.bitcoinppl.cove.testconfig

import kotlinx.coroutines.runBlocking
import org.bitcoinppl.cove.testconfig.ScriptedCloudStorageAccess.FixtureWalletDownloadResult
import org.bitcoinppl.cove.testconfig.ScriptedCloudStorageAccess.NamespaceResult
import org.bitcoinppl.cove.testconfig.ScriptedPasskeyProvider.Invocation
import org.bitcoinppl.cove.testconfig.ScriptedPasskeyProvider.Result
import org.bitcoinppl.cove_core.device.CloudAccessPolicy
import org.bitcoinppl.cove_core.device.CloudStorageException
import org.bitcoinppl.cove_core.device.PasskeyException
import org.bitcoinppl.cove_core.device.PasskeyFailureReason
import org.bitcoinppl.cove_core.device.PasskeyOperation
import org.bitcoinppl.cove_core.device.PasskeyRegistrationUser
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Test
import java.security.MessageDigest
import java.util.Locale
import java.util.concurrent.atomic.AtomicReference

class ScriptedProviderTest {
    private companion object {
        const val EXPECTED_NAMESPACE_REQUESTS = 4
        const val PRF_SALT_BYTE_COUNT = 32
    }

    @Before
    fun resetPasskeys() {
        ScriptedPasskeyProvider.reset()
    }

    @Test
    fun namespaceResultsQueueEmptyErrorSuccessAndRetainTheFinalResult() =
        runBlocking {
            ScriptedCloudStorageAccess.configureNamespaceResults(
                NamespaceResult.EMPTY,
                NamespaceResult.OFFLINE,
                NamespaceResult.BACKUP_FOUND,
            )

            assertTrue(ScriptedCloudStorageAccess.listNamespaces(CloudAccessPolicy.SILENT).isEmpty())

            val error =
                runCatching {
                    ScriptedCloudStorageAccess.listNamespaces(CloudAccessPolicy.CONSENT_ALLOWED)
                }.exceptionOrNull()
            assertTrue(error is CloudStorageException.Offline)

            val found = ScriptedCloudStorageAccess.listNamespaces(CloudAccessPolicy.CONSENT_ALLOWED)
            val repeated = ScriptedCloudStorageAccess.listNamespaces(CloudAccessPolicy.SILENT)

            assertEquals(found, repeated)
            assertEquals(1, found.size)
            assertEquals(
                EXPECTED_NAMESPACE_REQUESTS,
                ScriptedCloudStorageAccess.namespaceRequestCount(),
            )
        }

    @Test
    fun blockedNamespaceResultWaitsForAnExplicitRelease() {
        ScriptedCloudStorageAccess.configureNamespaceResults(
            NamespaceResult.EMPTY,
            blockedRequest = 1,
        )
        val result = AtomicReference<List<String>>()
        val request =
            Thread {
                result.set(
                    runBlocking {
                        ScriptedCloudStorageAccess.listNamespaces(CloudAccessPolicy.SILENT)
                    },
                )
            }
        request.start()

        assertTrue(ScriptedCloudStorageAccess.awaitNamespaceRequest())
        assertTrue(request.isAlive)

        ScriptedCloudStorageAccess.releaseBlockedNamespaceRequest()
        request.join()

        assertTrue(result.get().isEmpty())
    }

    @Test
    fun passkeyResultsKeepTypedPresentationFailuresAndCountCalls() {
        ScriptedPasskeyProvider.configureResults(
            Invocation.CREATE,
            Result.PRE_PRESENTATION_FAILURE,
            Result.SUCCESS,
        )
        val user = PasskeyRegistrationUser(byteArrayOf(1), "test", "Test")

        val first =
            runCatching {
                ScriptedPasskeyProvider.createPasskey("example.com", byteArrayOf(2), user)
            }.exceptionOrNull()
        assertTrue(first is PasskeyException.RequestFailed)
        first as PasskeyException.RequestFailed
        assertEquals(PasskeyOperation.REGISTRATION, first.operation)
        assertEquals(PasskeyFailureReason.PlatformAuthorizationFailed, first.reason)

        val created =
            ScriptedPasskeyProvider.createPasskey("example.com", byteArrayOf(3), user)
        assertArrayEquals("ui-test-passkey".encodeToByteArray(), created.credentialId)
        assertEquals(2, ScriptedPasskeyProvider.callCount(Invocation.CREATE))

        ScriptedPasskeyProvider.configureResults(
            Invocation.DISCOVER,
            Result.POST_PRESENTATION_FAILURE,
        )
        val discovery =
            runCatching {
                ScriptedPasskeyProvider.discoverAndAuthenticateWithPrf(
                    "example.com",
                    ByteArray(32),
                    byteArrayOf(4),
                )
            }.exceptionOrNull()
        assertTrue(discovery is PasskeyException.RequestFailed)
        discovery as PasskeyException.RequestFailed
        assertEquals(PasskeyOperation.DISCOVER_ASSERTION, discovery.operation)
        assertEquals(
            PasskeyFailureReason.PlatformAuthorizationFailedAfterPresentation,
            discovery.reason,
        )
        assertEquals(1, ScriptedPasskeyProvider.callCount(Invocation.DISCOVER))
    }

    @Test
    fun productionFixtureBytesMatchTheGeneratorOutput() {
        assertEquals(
            "010c829b67265561d7b6cb6be6a7ada86681723495dd3523c40a1e819156a64a",
            ScriptedCloudBackupFixture.masterWrapper.sha256(),
        )
        assertEquals(
            "de30ced3f7d25190e48a6f7465625e8172e230772ed6bb8b7e6ff017a75c0038",
            ScriptedCloudBackupFixture.walletWrapper.sha256(),
        )
        assertEquals(
            "e993e3215f98ca752300a34829070d225dde3061e042ea452cd7e7f4fa4c8748",
            requireNotNull(
                ScriptedCloudBackupFixture.walletWrapper(
                    ScriptedCloudBackupFixture.WALLET_TWO_RECORD_ID,
                ),
            ).sha256(),
        )
        assertEquals(
            "9dd1bebd7d659f61b14eb852b08c7098e11b69b65979cffa06540ff646944aa7",
            requireNotNull(
                ScriptedCloudBackupFixture.walletWrapper(
                    ScriptedCloudBackupFixture.WALLET_THREE_RECORD_ID,
                ),
            ).sha256(),
        )
        assertTrue(
            ScriptedCloudBackupFixture.masterWrapper
                .decodeToString()
                .contains("\"prf_salt\":\"${"07".repeat(PRF_SALT_BYTE_COUNT)}\""),
        )
    }

    @Test
    fun fixtureWalletInventoryAndQueuedDownloadsAreRecordLocal() =
        runBlocking {
            val recordId = ScriptedCloudBackupFixture.WALLET_TWO_RECORD_ID
            ScriptedCloudStorageAccess.configureProductionFixtureRestore()
            ScriptedCloudStorageAccess.exposeAllProductionFixtureWallets()
            ScriptedCloudStorageAccess.configureFixtureWalletDownloads(
                recordId,
                FixtureWalletDownloadResult.CORRUPT,
                FixtureWalletDownloadResult.VALID,
            )

            assertEquals(
                ScriptedCloudBackupFixture.WALLET_RECORD_IDS.map { recordId ->
                    "wallet-$recordId.json"
                },
                ScriptedCloudStorageAccess.listWalletFiles(
                    ScriptedCloudBackupFixture.NAMESPACE,
                    CloudAccessPolicy.SILENT,
                ),
            )

            val corrupt =
                ScriptedCloudStorageAccess.downloadWalletBackup(
                    ScriptedCloudBackupFixture.NAMESPACE,
                    recordId,
                    emptyList(),
                    CloudAccessPolicy.SILENT,
                )
            val valid =
                ScriptedCloudStorageAccess.downloadWalletBackup(
                    ScriptedCloudBackupFixture.NAMESPACE,
                    recordId,
                    emptyList(),
                    CloudAccessPolicy.SILENT,
                )
            ScriptedCloudStorageAccess.configureFixtureWalletDownloadDefault(
                recordId,
                FixtureWalletDownloadResult.CORRUPT,
            )
            val corruptDefault =
                ScriptedCloudStorageAccess.downloadWalletBackup(
                    ScriptedCloudBackupFixture.NAMESPACE,
                    recordId,
                    emptyList(),
                    CloudAccessPolicy.SILENT,
                )

            assertTrue(!corrupt.contentEquals(valid))
            assertArrayEquals(corrupt, corruptDefault)
            assertEquals(3, ScriptedCloudStorageAccess.walletDownloadCount(recordId))
            assertArrayEquals(
                ScriptedCloudBackupFixture.walletWrapper(recordId),
                valid,
            )
        }

    private fun ByteArray.sha256(): String =
        MessageDigest
            .getInstance("SHA-256")
            .digest(this)
            .joinToString("") { byte -> "%02x".format(Locale.ROOT, byte) }
}
