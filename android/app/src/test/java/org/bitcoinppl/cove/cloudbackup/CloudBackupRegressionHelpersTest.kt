package org.bitcoinppl.cove.cloudbackup

import org.bitcoinppl.cove_core.CloudBackupStatus
import org.bitcoinppl.cove_core.CloudOnlyState
import org.bitcoinppl.cove_core.DeepVerificationFailure
import org.bitcoinppl.cove_core.VerificationFailureKind
import org.bitcoinppl.cove_core.VerificationState
import org.bitcoinppl.cove_core.device.CloudSyncHealth
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class CloudBackupRegressionHelpersTest {
    @Test
    fun transientStatusesDoNotForceCloudBackupEnabled() {
        val transientStatuses =
            listOf(
                CloudBackupStatus.Enabling,
                CloudBackupStatus.Error("failed"),
                CloudBackupStatus.UnsupportedPasskeyProvider,
            )

        transientStatuses.forEach { status ->
            val enabled =
                cloudBackupEnabledForStatus(
                    status = status,
                    currentValue = false,
                    readPersistedState = { false },
                )
            assertFalse(enabled)
        }
    }

    @Test
    fun persistedEnabledStateWinsForConfiguredStatuses() {
        assertTrue(
            cloudBackupEnabledForStatus(
                status = CloudBackupStatus.Enabled,
                currentValue = false,
                readPersistedState = { true },
            ),
        )
        assertFalse(
            cloudBackupEnabledForStatus(
                status = CloudBackupStatus.Disabled,
                currentValue = true,
                readPersistedState = { false },
            ),
        )
    }

    @Test
    fun syncHealthRequiresUploadedBackupFiles() {
        val hasUploadedBackupFiles: (List<String>) -> Boolean = { fileNames ->
            fileNames.any { it == "master-key.json" || (it.startsWith("wallet-") && it.endsWith(".json")) }
        }

        assertEquals(
            CloudSyncHealth.NoFiles,
            syncHealthForNamespaceFiles(
                namespaceFiles = emptyList(),
                hasUploadedBackupFiles = hasUploadedBackupFiles,
            ),
        )
        assertEquals(
            CloudSyncHealth.NoFiles,
            syncHealthForNamespaceFiles(
                namespaceFiles =
                    listOf(
                    emptyList(),
                    listOf("notes.txt", "placeholder"),
                ),
                hasUploadedBackupFiles = hasUploadedBackupFiles,
            ),
        )
        assertEquals(
            CloudSyncHealth.AllUploaded,
            syncHealthForNamespaceFiles(
                namespaceFiles = listOf(listOf("master-key.json")),
                hasUploadedBackupFiles = hasUploadedBackupFiles,
            ),
        )
        assertEquals(
            CloudSyncHealth.AllUploaded,
            syncHealthForNamespaceFiles(
                namespaceFiles = listOf(listOf("wallet-wallet-record.json")),
                hasUploadedBackupFiles = hasUploadedBackupFiles,
            ),
        )
    }

    @Test
    fun cancelledVerificationKeepsDetailAndFallbackRecoveryReachable() {
        assertEquals(
            CloudBackupDetailBodyState.DETAIL,
            cloudBackupDetailBodyState(
                status = CloudBackupStatus.Enabled,
                verification = VerificationState.Cancelled,
                hasDetail = true,
            ),
        )
        assertEquals(
            CloudBackupDetailBodyState.CANCELLED_RECOVERY,
            cloudBackupDetailBodyState(
                status = CloudBackupStatus.Enabled,
                verification = VerificationState.Cancelled,
                hasDetail = false,
            ),
        )
    }

    @Test
    fun failedVerificationWithoutDetailShowsFallbackVerificationSection() {
        val bodyState =
            cloudBackupDetailBodyState(
                status = CloudBackupStatus.Enabled,
                verification =
                    VerificationState.Failed(
                        DeepVerificationFailure(
                            kind = VerificationFailureKind.Retry,
                            message = "Drive unavailable",
                            detail = null,
                        ),
                    ),
                hasDetail = false,
            )

        assertNull(bodyState)
        assertTrue(shouldShowFallbackVerificationSection(bodyState))
        assertFalse(shouldShowFallbackVerificationSection(CloudBackupDetailBodyState.DETAIL))
    }

    @Test
    fun cloudOnlyAutoFetchOnlyRunsFromNotFetched() {
        assertTrue(shouldFetchCloudOnly(CloudOnlyState.NotFetched))
        assertFalse(shouldFetchCloudOnly(CloudOnlyState.Loading))
    }
}
