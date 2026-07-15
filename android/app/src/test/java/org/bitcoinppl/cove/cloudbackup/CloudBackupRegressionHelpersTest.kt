package org.bitcoinppl.cove.cloudbackup

import kotlinx.coroutines.delay
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeout
import org.bitcoinppl.cove_core.CloudBackupConfiguredState
import org.bitcoinppl.cove_core.CloudBackupDetail
import org.bitcoinppl.cove_core.CloudBackupDetailState
import org.bitcoinppl.cove_core.CloudBackupDestructiveOperationState
import org.bitcoinppl.cove_core.CloudBackupEnableContext
import org.bitcoinppl.cove_core.CloudBackupEnableFlow
import org.bitcoinppl.cove_core.CloudBackupInventoryIncompleteReason
import org.bitcoinppl.cove_core.CloudBackupLifecycle
import org.bitcoinppl.cove_core.CloudBackupManagerAction
import org.bitcoinppl.cove_core.CloudBackupOtherBackupsState
import org.bitcoinppl.cove_core.CloudBackupOtherBackupsSummary
import org.bitcoinppl.cove_core.CloudBackupPasskeyChoiceIntent
import org.bitcoinppl.cove_core.CloudBackupPasskeyState
import org.bitcoinppl.cove_core.CloudBackupProgress
import org.bitcoinppl.cove_core.CloudBackupReconcileMessage
import org.bitcoinppl.cove_core.CloudBackupRootPrompt
import org.bitcoinppl.cove_core.CloudBackupRestoreAllState
import org.bitcoinppl.cove_core.CloudBackupState
import org.bitcoinppl.cove_core.CloudBackupSettingsRowStatus
import org.bitcoinppl.cove_core.CloudBackupSyncState
import org.bitcoinppl.cove_core.CloudBackupVerificationPresentation
import org.bitcoinppl.cove_core.CloudBackupVerificationSource
import org.bitcoinppl.cove_core.CloudBackupVerificationState
import org.bitcoinppl.cove_core.CloudOnlyState
import org.bitcoinppl.cove_core.CloudOnlyOperation
import org.bitcoinppl.cove_core.DeepVerificationFailure
import org.bitcoinppl.cove_core.LoadedCloudBackupDetail
import org.bitcoinppl.cove_core.OtherBackupsOperation
import org.bitcoinppl.cove_core.SavedPasskeyConfirmationMode
import org.bitcoinppl.cove_core.device.CloudSyncHealth
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class CloudBackupRegressionHelpersTest {
    @Test
    fun scP04EveryEnableStateProjectsItsExpectedBusyPhase() {
        val context = manualEnableContext()
        val hidden = CloudBackupVerificationPresentation.Hidden(null)
        val checkingPasskey = "Checking that your passkey is available..."
        val creatingBackup = "Creating your encrypted backup..."
        val states =
            listOf<Pair<CloudBackupEnableFlow?, String>>(
                CloudBackupEnableFlow.DiscoveringExistingBackup to creatingBackup,
                CloudBackupEnableFlow.AwaitingForceNewConfirmation(context, null) to creatingBackup,
                CloudBackupEnableFlow.AwaitingPasskeyChoice(
                    CloudBackupPasskeyChoiceIntent.Enable(context, null),
                ) to creatingBackup,
                CloudBackupEnableFlow.CreatingPasskey to "Creating your passkey...",
                CloudBackupEnableFlow.AwaitingSavedPasskeyConfirmation(
                    SavedPasskeyConfirmationMode.AUTOMATIC,
                ) to checkingPasskey,
                CloudBackupEnableFlow.AwaitingSavedPasskeyConfirmation(
                    SavedPasskeyConfirmationMode.MANUAL,
                ) to checkingPasskey,
                CloudBackupEnableFlow.ConfirmingSavedPasskey to "Confirming your passkey...",
                CloudBackupEnableFlow.UploadingInitialBackup(null) to creatingBackup,
                CloudBackupEnableFlow.RetryingUploadWithStagedMaterial(null) to creatingBackup,
                CloudBackupEnableFlow.WaitingForPasskeyAvailability to checkingPasskey,
                null to creatingBackup,
            )

        states.forEach { (state, expectedTitle) ->
            assertEquals(expectedTitle, cloudBackupEnableBusyCopy(state, hidden).title)
        }
    }

    @Test
    fun enableBusyCopyProjectsUploadCountsForInitialAndRetryFlows() {
        val progress = CloudBackupProgress(completed = 2u, total = 5u)
        val hidden = CloudBackupVerificationPresentation.Hidden(null)

        listOf(
            CloudBackupEnableFlow.UploadingInitialBackup(progress),
            CloudBackupEnableFlow.RetryingUploadWithStagedMaterial(progress),
        ).forEach { flow ->
            val copy = cloudBackupEnableBusyCopy(flow, hidden)

            assertEquals("Creating your encrypted backup...", copy.title)
            assertEquals("Completed 2 of 5", copy.subtitle)
            assertEquals(progress, copy.progress)
        }
    }

    @Test
    fun enableBusyCopyPreservesPhaseAndBackgroundConfirmationCopy() {
        val hidden = CloudBackupVerificationPresentation.Hidden(null)
        assertEquals(
            "Confirming your passkey...",
            cloudBackupEnableBusyCopy(CloudBackupEnableFlow.ConfirmingSavedPasskey, hidden).title,
        )
        assertEquals(
            "Cloud Backup will continue automatically",
            cloudBackupEnableBusyCopy(
                CloudBackupEnableFlow.UploadingInitialBackup(null),
                hidden,
            ).subtitle,
        )

        val background =
            cloudBackupEnableBusyCopy(
                null,
                CloudBackupVerificationPresentation.BackgroundConfirming(
                    CloudBackupVerificationSource.ONBOARDING,
                ),
            )
        assertEquals("Confirming your encrypted backup...", background.title)
        assertTrue(background.subtitle.contains("visible in Google Drive"))
        assertTrue(background.subtitle.contains("continues in the background"))
        assertNull(background.progress)
    }

    @Test
    fun enableCompletionIsRetainedAndConsumedByIdentity() =
        runBlocking {
            val manager = cloudBackupManager()
            val context =
                CloudBackupEnableContext(
                    savedPasskeyConfirmation = SavedPasskeyConfirmationMode.AUTOMATIC,
                    verificationSource = CloudBackupVerificationSource.ONBOARDING,
                )

            try {
                manager.reconcile(CloudBackupReconcileMessage.EnableCompleted(context))
                withTimeout(1_000) {
                    while (manager.enableCompletion == null) {
                        delay(10)
                    }
                }

                val completion = requireNotNull(manager.enableCompletion)
                assertEquals(context, completion.item)

                manager.consumeEnableCompletion(completion)
                assertNull(manager.enableCompletion)
            } finally {
                manager.close()
            }
        }

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
                                    retryAction = null,
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
    fun failedInventoryShowsRetryStateInsteadOfLoadingOrZero() {
        val manager =
            cloudBackupManager(
                detail =
                    CloudBackupDetailState.Failed(
                        reason = CloudBackupInventoryIncompleteReason.OFFLINE,
                        error = "Drive inventory is unavailable",
                        retained = null,
                    ),
            )

        assertEquals(
            CloudBackupDetailBodyState.INVENTORY_FAILED,
            cloudBackupDetailBodyState(manager = manager, hasDetail = false),
        )
        assertEquals("Drive inventory is unavailable", manager.detailError)
        assertFalse(manager.isDetailInventoryComplete)
    }

    @Test
    fun scP04EveryInventoryStateRetainsRowsButOnlyCompleteEnablesActions() {
        val detail =
            CloudBackupDetail(
                lastSync = null,
                upToDate = emptyList(),
                needsSync = emptyList(),
                cloudOnlyCount = 1u,
                otherBackups =
                    CloudBackupOtherBackupsState.Loaded(
                        CloudBackupOtherBackupsSummary(
                            namespaceCount = 0u,
                            walletCount = 0u,
                            passkeyHints = emptyList(),
                        ),
                    ),
            )
        val loaded =
            LoadedCloudBackupDetail(
                detail = detail,
                cloudOnly = CloudOnlyState.NotFetched,
                cloudOnlyOperation = CloudOnlyOperation.Idle,
                otherBackupsOperation = OtherBackupsOperation.Idle,
            )

        val notLoaded = cloudBackupManager()
        assertNull(notLoaded.detail)
        assertFalse(notLoaded.isDetailInventoryChecking)
        assertFalse(notLoaded.isDetailInventoryComplete)

        val checking = cloudBackupManager(detail = CloudBackupDetailState.Checking(retained = loaded))
        assertEquals(detail, checking.detail)
        assertTrue(checking.isDetailInventoryChecking)
        assertFalse(checking.isDetailInventoryComplete)

        val failed =
            cloudBackupManager(
                detail =
                    CloudBackupDetailState.Failed(
                        reason = CloudBackupInventoryIncompleteReason.OFFLINE,
                        error = "Drive inventory is unavailable",
                        retained = loaded,
                    ),
            )
        assertEquals(detail, failed.detail)
        assertEquals("Drive inventory is unavailable", failed.detailError)
        assertFalse(failed.isDetailInventoryChecking)
        assertFalse(failed.isDetailInventoryComplete)

        val complete = cloudBackupManager(detail = CloudBackupDetailState.Complete(state = loaded))
        assertEquals(detail, complete.detail)
        assertFalse(complete.isDetailInventoryChecking)
        assertTrue(complete.isDetailInventoryComplete)
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
    fun existingOnlyPasskeyChoiceDoesNotOfferNewBackup() {
        val presentation =
            cloudBackupPasskeyChoicePresentation(
                CloudBackupPasskeyChoiceIntent.EnableExistingPasskeyOnly(manualEnableContext(), null),
            )

        assertNull(presentation.secondaryActionTitle)
        assertTrue(presentation.message.contains("existing passkey"))
    }

    @Test
    fun normalEnableOffersStartNewBackup() {
        val presentation =
            cloudBackupPasskeyChoicePresentation(
                CloudBackupPasskeyChoiceIntent.Enable(manualEnableContext(), null),
            )

        assertEquals("Start a New Backup", presentation.secondaryActionTitle)
    }

    @Test
    fun repairPasskeyChoiceKeepsCreateNewPasskeyAction() {
        val presentation =
            cloudBackupPasskeyChoicePresentation(CloudBackupPasskeyChoiceIntent.RepairPasskey)

        assertEquals("Create New Passkey", presentation.secondaryActionTitle)
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
        detail: CloudBackupDetailState = CloudBackupDetailState.NotLoaded,
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
                            detail = detail,
                            restoreAll = CloudBackupRestoreAllState.NotShown,
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
