package org.bitcoinppl.cove.cloudbackup

import org.bitcoinppl.cove_core.CloudBackupPasskeyChoiceFlow
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
        val masterKeyFileName = "master-key.json"
        val isWalletFile: (String) -> Boolean = { fileName ->
            fileName.startsWith("wallet-") && fileName.endsWith(".json")
        }
        val hasUploadedBackupFileNames: (List<String>) -> Boolean = { fileNames ->
            hasUploadedBackupFiles(
                fileNames = fileNames,
                masterKeyFileName = masterKeyFileName,
                isWalletFile = isWalletFile,
            )
        }
        val hasMasterKeyBackupFile: (List<String>) -> Boolean = { fileNames ->
            hasMasterKeyBackup(
                fileNames = fileNames,
                masterKeyFileName = masterKeyFileName,
            )
        }

        assertEquals(
            CloudSyncHealth.NoFiles,
            syncHealthForNamespaceFiles(
                namespaceFiles = emptyList(),
                hasUploadedBackupFiles = hasUploadedBackupFileNames,
                hasMasterKeyBackup = hasMasterKeyBackupFile,
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
                hasUploadedBackupFiles = hasUploadedBackupFileNames,
                hasMasterKeyBackup = hasMasterKeyBackupFile,
            ),
        )
        assertEquals(
            CloudSyncHealth.AllUploaded,
            syncHealthForNamespaceFiles(
                namespaceFiles = listOf(listOf(masterKeyFileName)),
                hasUploadedBackupFiles = hasUploadedBackupFileNames,
                hasMasterKeyBackup = hasMasterKeyBackupFile,
            ),
        )
        assertEquals(
            CloudSyncHealth.Failed("cloud backup is incomplete"),
            syncHealthForNamespaceFiles(
                namespaceFiles = listOf(listOf("wallet-wallet-record.json")),
                hasUploadedBackupFiles = hasUploadedBackupFileNames,
                hasMasterKeyBackup = hasMasterKeyBackupFile,
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

    @Test
    fun decoyModeBlocksAllCloudBackupRootPresentations() {
        val context =
            CloudBackupPresentationContext(
                isActivityResumed = true,
                isUnlocked = true,
                isInDecoyMode = true,
                isCoverPresented = false,
            )

        val presentations =
            listOf(
                CloudBackupRootPresentation.ExistingBackupFound,
                CloudBackupRootPresentation.PasskeyChoice(CloudBackupPasskeyChoiceFlow.ENABLE),
                CloudBackupRootPresentation.MissingPasskeyReminder,
                CloudBackupRootPresentation.VerificationPrompt,
            )

        presentations.forEach { presentation ->
            assertFalse(
                isCloudBackupPresentationPresentable(
                    presentation = presentation,
                    context = context,
                    hasBlockers = false,
                ),
            )
        }
        assertTrue(
            isCloudBackupPresentationPresentable(
                presentation = CloudBackupRootPresentation.ExistingBackupFound,
                context = context.copy(isInDecoyMode = false),
                hasBlockers = false,
            ),
        )
    }
}
