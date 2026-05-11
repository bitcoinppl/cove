package org.bitcoinppl.cove.cloudbackup

import androidx.compose.runtime.mutableStateOf
import org.bitcoinppl.cove_core.CloudBackupConfiguredState
import org.bitcoinppl.cove_core.CloudBackupDetailState
import org.bitcoinppl.cove_core.CloudBackupEnableContext
import org.bitcoinppl.cove_core.CloudBackupLifecycle
import org.bitcoinppl.cove_core.CloudBackupPasskeyChoiceIntent
import org.bitcoinppl.cove_core.CloudBackupPasskeyState
import org.bitcoinppl.cove_core.CloudBackupRootPrompt
import org.bitcoinppl.cove_core.CloudBackupState
import org.bitcoinppl.cove_core.CloudBackupSyncState
import org.bitcoinppl.cove_core.CloudBackupVerificationPresentation
import org.bitcoinppl.cove_core.CloudBackupVerificationSource
import org.bitcoinppl.cove_core.CloudBackupVerificationState
import org.bitcoinppl.cove_core.CloudOnlyState
import org.bitcoinppl.cove_core.DeepVerificationFailure
import org.bitcoinppl.cove_core.SavedPasskeyConfirmationMode
import org.bitcoinppl.cove_core.device.CloudSyncHealth
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import sun.misc.Unsafe

class CloudBackupRegressionHelpersTest {
    @Test
    fun notVerifiedStateKeepsDetailReachable() {
        assertEquals(
            CloudBackupDetailBodyState.DETAIL,
            cloudBackupDetailBodyState(
                manager = cloudBackupManager(verification = CloudBackupVerificationState.NotVerified),
                hasDetail = true,
            ),
        )
        assertEquals(
            CloudBackupDetailBodyState.LOADING,
            cloudBackupDetailBodyState(
                manager = cloudBackupManager(verification = CloudBackupVerificationState.NotVerified),
                hasDetail = false,
            ),
        )
    }

    @Test
    fun failedVerificationWithoutDetailShowsFallbackVerificationSection() {
        val bodyState =
            cloudBackupDetailBodyState(
                manager =
                    cloudBackupManager(
                        verification =
                            CloudBackupVerificationState.Failed(
                                DeepVerificationFailure.Retry(
                                    message = "Drive unavailable",
                                    detail = null,
                                    retryContext = null,
                                ),
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
    fun pendingUploadConfirmationDoesNotReplaceDetailContent() {
        assertEquals(
            CloudBackupDetailBodyState.DETAIL,
            cloudBackupDetailBodyState(
                manager =
                    cloudBackupManager(
                        verification = CloudBackupVerificationState.AwaitingUploadConfirmation,
                    ),
                hasDetail = true,
            ),
        )
        assertTrue(
            shouldShowPendingUploadConfirmationStatus(
                cloudBackupManager(
                    verification = CloudBackupVerificationState.AwaitingUploadConfirmation,
                ),
            ),
        )
    }

    @Test
    fun interactiveVerificationKeepsVerifyingBody() {
        assertEquals(
            CloudBackupDetailBodyState.VERIFYING,
            cloudBackupDetailBodyState(
                manager = cloudBackupManager(verification = CloudBackupVerificationState.Running),
                hasDetail = true,
            ),
        )
    }

    @Test
    fun pendingUploadConfirmationWithoutDetailKeepsBackgroundLoadingBody() {
        assertEquals(
            CloudBackupDetailBodyState.LOADING,
            cloudBackupDetailBodyState(
                manager =
                    cloudBackupManager(
                        verification = CloudBackupVerificationState.AwaitingUploadConfirmation,
                    ),
                hasDetail = false,
            ),
        )
    }

    @Test
    fun blockedPendingUploadAuthorizationWithoutDetailShowsAuthorizationBody() {
        assertEquals(
            CloudBackupDetailBodyState.AUTHORIZATION_BLOCKED,
            cloudBackupDetailBodyState(
                manager =
                    cloudBackupManager(
                        verification = CloudBackupVerificationState.AwaitingUploadConfirmation,
                        sync = CloudBackupSyncState.Blocked("authorization required"),
                    ),
                hasDetail = false,
            ),
        )
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
                CloudBackupRootPresentation.ExistingBackupFound(manualEnableContext(), null),
                CloudBackupRootPresentation.PasskeyChoice(
                    CloudBackupPasskeyChoiceIntent.Enable(manualEnableContext(), null),
                ),
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
                presentation = CloudBackupRootPresentation.ExistingBackupFound(manualEnableContext(), null),
                context = context.copy(isInDecoyMode = false),
                hasBlockers = false,
            ),
        )
    }

    private fun manualEnableContext(): CloudBackupEnableContext =
        CloudBackupEnableContext(
            savedPasskeyConfirmation = SavedPasskeyConfirmationMode.MANUAL,
            verificationSource = CloudBackupVerificationSource.SETTINGS,
        )

    private fun cloudBackupManager(
        passkey: CloudBackupPasskeyState = CloudBackupPasskeyState.Available,
        verification: CloudBackupVerificationState = CloudBackupVerificationState.NotVerified,
        sync: CloudBackupSyncState = CloudBackupSyncState.Idle,
    ): CloudBackupManager {
        val manager = unsafe.allocateInstance(CloudBackupManager::class.java) as CloudBackupManager
        val state =
            CloudBackupState(
                lifecycle =
                    CloudBackupLifecycle.Configured(
                        CloudBackupConfiguredState(
                            passkey = passkey,
                            verification = verification,
                            sync = sync,
                            detail = CloudBackupDetailState.NotLoaded,
                            lastRestoreReport = null,
                            rootPrompt = CloudBackupRootPrompt.None,
                            syncHealth = CloudSyncHealth.Unknown,
                            verificationPresentation = CloudBackupVerificationPresentation.Hidden(null),
                        ),
                    ),
            )

        CloudBackupManager::class.java
            .getDeclaredField("state\$delegate")
            .apply {
                isAccessible = true
                set(manager, mutableStateOf(state))
            }

        return manager
    }

    private val unsafe: Unsafe
        get() {
            val field = Unsafe::class.java.getDeclaredField("theUnsafe")
            field.isAccessible = true
            return field.get(null) as Unsafe
        }
}
