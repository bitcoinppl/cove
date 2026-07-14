package org.bitcoinppl.cove.cloudbackup

import org.bitcoinppl.cove_core.CloudBackupConfiguredState
import org.bitcoinppl.cove_core.CloudBackupDetailState
import org.bitcoinppl.cove_core.CloudBackupDestructiveOperationState
import org.bitcoinppl.cove_core.CloudBackupEnableContext
import org.bitcoinppl.cove_core.CloudBackupLifecycle
import org.bitcoinppl.cove_core.CloudBackupManagerAction
import org.bitcoinppl.cove_core.CloudBackupPasskeyChoiceIntent
import org.bitcoinppl.cove_core.CloudBackupPasskeyState
import org.bitcoinppl.cove_core.CloudBackupRootPrompt
import org.bitcoinppl.cove_core.CloudBackupState
import org.bitcoinppl.cove_core.CloudBackupSettingsRowStatus
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
    fun cancelledVerificationShowsRecoveryInsteadOfLoadedDetail() {
        assertEquals(
            CloudBackupDetailBodyState.CANCELLED,
            cloudBackupDetailBodyState(
                manager = cloudBackupManager(verification = CloudBackupVerificationState.Cancelled),
                hasDetail = true,
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
    fun onboardingPolicySuppressesGenericCloudBackupRootPrompts() {
        val context =
            presentableContext(
                presentationPolicy = CloudBackupPresentationPolicy.ONBOARDING,
            )

        assertFalse(
            isCloudBackupPresentationPresentable(
                presentation = CloudBackupRootPresentation.VerificationPrompt,
                context = context,
                hasBlockers = false,
            ),
        )
        assertFalse(
            isCloudBackupPresentationPresentable(
                presentation = CloudBackupRootPresentation.MissingPasskeyReminder,
                context = context,
                hasBlockers = false,
            ),
        )
    }

    @Test
    fun onboardingPolicyAllowsCloudBackupEnablePrompts() {
        val context =
            presentableContext(
                presentationPolicy = CloudBackupPresentationPolicy.ONBOARDING,
            )

        assertTrue(
            isCloudBackupPresentationPresentable(
                presentation = CloudBackupRootPresentation.ExistingBackupFound(manualEnableContext(), null),
                context = context,
                hasBlockers = false,
            ),
        )
        assertTrue(
            isCloudBackupPresentationPresentable(
                presentation =
                    CloudBackupRootPresentation.PasskeyChoice(
                        CloudBackupPasskeyChoiceIntent.Enable(manualEnableContext(), null),
                    ),
                context = context,
                hasBlockers = false,
            ),
        )
    }

    @Test
    fun unsettledNavigationBlocksCloudBackupRootPrompts() {
        assertFalse(
            isCloudBackupPresentationPresentable(
                presentation = CloudBackupRootPresentation.VerificationPrompt,
                context =
                    presentableContext(
                        presentationPolicy = CloudBackupPresentationPolicy.REQUIRES_UNLOCKED_AUTH,
                    ).copy(isNavigationSettled = false),
                hasBlockers = false,
            ),
        )
    }

    @Test
    fun rootPromptVerificationResultsProduceFeedback() {
        assertEquals(
            CloudBackupVerificationFeedback.SuccessFloater("Cloud Backup Verified"),
            cloudBackupVerificationFeedback(
                CloudBackupVerificationPresentation.Completed(
                    CloudBackupVerificationSource.ROOT_PROMPT,
                ),
            ),
        )
        assertEquals(
            CloudBackupVerificationFeedback.FailureAlert(
                title = "Cloud Backup Verification Failed",
                message = "Drive unavailable",
            ),
            cloudBackupVerificationFeedback(
                CloudBackupVerificationPresentation.Failed(
                    source = CloudBackupVerificationSource.ROOT_PROMPT,
                    message = "Drive unavailable",
                ),
            ),
        )
    }

    @Test
    fun nonRootPromptVerificationResultsDoNotProduceFeedback() {
        assertNull(
            cloudBackupVerificationFeedback(
                CloudBackupVerificationPresentation.Completed(
                    CloudBackupVerificationSource.SETTINGS,
                ),
            ),
        )
        assertNull(
            cloudBackupVerificationFeedback(
                CloudBackupVerificationPresentation.Failed(
                    source = CloudBackupVerificationSource.ONBOARDING,
                    message = "Drive unavailable",
                ),
            ),
        )
    }

    @Test
    fun settingsEnableStartsManualPasskeyChoicePrompt() {
        val action = settingsEnableCloudBackupPrompt()
        assertTrue(action is CloudBackupManagerAction.PromptEnablePasskeyChoice)

        val context = (action as CloudBackupManagerAction.PromptEnablePasskeyChoice).v1

        assertEquals(SavedPasskeyConfirmationMode.MANUAL, context.savedPasskeyConfirmation)
        assertEquals(CloudBackupVerificationSource.SETTINGS, context.verificationSource)
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

    private fun presentableContext(
        presentationPolicy: CloudBackupPresentationPolicy,
    ): CloudBackupPresentationContext =
        CloudBackupPresentationContext(
            isActivityResumed = true,
            isUnlocked = true,
            isCoverPresented = false,
            presentationPolicy = presentationPolicy,
        )

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
        val state =
            CloudBackupState(
                lifecycle =
                    CloudBackupLifecycle.Configured(
                        CloudBackupConfiguredState(
                            passkey = passkey,
                            verification = verification,
                            sync = sync,
                            destructiveOperation = CloudBackupDestructiveOperationState.Idle,
                            detail = CloudBackupDetailState.NotLoaded,
                            rootPrompt = CloudBackupRootPrompt.None,
                            syncHealth = CloudSyncHealth.Unknown,
                            verificationPresentation = CloudBackupVerificationPresentation.Hidden(null),
                        ),
                    ),
                settingsRowStatus = CloudBackupSettingsRowStatus.CheckingSync,
            )

        return CloudBackupManager(state)
    }
}
