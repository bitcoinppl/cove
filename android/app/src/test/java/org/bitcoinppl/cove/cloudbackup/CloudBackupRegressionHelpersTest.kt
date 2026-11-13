package org.bitcoinppl.cove.cloudbackup

import kotlinx.coroutines.delay
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeout
import org.bitcoinppl.cove_core.CloudBackupActions
import org.bitcoinppl.cove_core.CloudBackupEnableContext
import org.bitcoinppl.cove_core.CloudBackupEnablePromptChoice
import org.bitcoinppl.cove_core.CloudBackupEnableFlow
import org.bitcoinppl.cove_core.CloudBackupManagerAction
import org.bitcoinppl.cove_core.CloudBackupPasskeyChoiceIntent
import org.bitcoinppl.cove_core.CloudBackupProgress
import org.bitcoinppl.cove_core.CloudBackupReconcileMessage
import org.bitcoinppl.cove_core.CloudBackupRootPrompt
import org.bitcoinppl.cove_core.CloudBackupScreen
import org.bitcoinppl.cove_core.CloudBackupScreenAction
import org.bitcoinppl.cove_core.CloudBackupScreenBody
import org.bitcoinppl.cove_core.CloudBackupSettingsRowStatus
import org.bitcoinppl.cove_core.CloudBackupVerificationPresentation
import org.bitcoinppl.cove_core.CloudBackupVerificationSource
import org.bitcoinppl.cove_core.SavedPasskeyConfirmationMode
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class CloudBackupRegressionHelpersTest {
    @Test
    fun everyEnableStateProjectsItsExpectedBusyPhase() {
        val context = manualEnableContext()
        val hidden = CloudBackupVerificationPresentation.Hidden(null)
        val checkingPasskey = "Checking that your passkey is available..."
        val checkingExistingBackups = "Checking for existing backups..."
        val creatingBackup = "Creating your encrypted backup..."
        val states =
            listOf<Pair<CloudBackupEnableFlow?, String>>(
                CloudBackupEnableFlow.DiscoveringExistingBackup to checkingExistingBackups,
                CloudBackupEnableFlow.AwaitingForceNewConfirmation(context, null) to checkingExistingBackups,
                CloudBackupEnableFlow.AwaitingPasskeyChoice(
                    CloudBackupPasskeyChoiceIntent.Enable(context, null),
                ) to checkingExistingBackups,
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
                null to checkingExistingBackups,
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
    fun enableBusyCopyPreservesBackgroundConfirmationCopy() {
        val hidden = CloudBackupVerificationPresentation.Hidden(null)
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
        val intent = CloudBackupPasskeyChoiceIntent.EnableExistingPasskeyOnly(manualEnableContext(), null)
        val choices = passkeyChoicePromptChoices(intent)

        assertEquals(
            listOf(CloudBackupScreenAction.ACCEPT_ENABLE_PROMPT_USE_EXISTING),
            choices.map { it.action },
        )
        assertTrue(cloudBackupPasskeyChoicePresentation(intent).message.contains("existing passkey"))
    }

    @Test
    fun normalEnableOffersStartNewBackup() {
        val choices =
            passkeyChoicePromptChoices(CloudBackupPasskeyChoiceIntent.Enable(manualEnableContext(), null))

        assertEquals(
            listOf("Use Existing Passkey", "Start a New Backup"),
            choices.map { it.title },
        )
        assertEquals(
            CloudBackupManagerAction.AcceptEnablePrompt(CloudBackupEnablePromptChoice.CREATE_NEW),
            choices.last().intent,
        )
    }

    @Test
    fun repairPasskeyChoiceKeepsCreateNewPasskeyAction() {
        val choices = passkeyChoicePromptChoices(CloudBackupPasskeyChoiceIntent.RepairPasskey)

        assertEquals("Create New Passkey", choices.last().title)
        assertEquals(CloudBackupManagerAction.RepairPasskeyNoDiscovery, choices.last().intent)
        assertEquals(
            listOf(CloudBackupScreenAction.REPAIR_PASSKEY, CloudBackupScreenAction.REPAIR_PASSKEY),
            choices.map { it.action },
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

    private fun cloudBackupManager(): CloudBackupManager =
        CloudBackupManager(
            CloudBackupScreen(
                body = CloudBackupScreenBody.Disabled,
                settingsRow = CloudBackupSettingsRowStatus.Disabled,
                healNotice = null,
                lastOutcome = null,
                rootPrompt = CloudBackupRootPrompt.None,
                actions = CloudBackupActions(emptyList()),
            ),
        )
}
