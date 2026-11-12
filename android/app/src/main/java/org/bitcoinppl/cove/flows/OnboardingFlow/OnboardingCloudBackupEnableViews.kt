package org.bitcoinppl.cove.flows.OnboardingFlow

import androidx.activity.compose.BackHandler
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Check
import androidx.compose.material.icons.filled.CloudDownload
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import org.bitcoinppl.cove.cloudbackup.CloudBackupEnableBusyOverlay
import org.bitcoinppl.cove.cloudbackup.CloudBackupEnableOnboardingContext
import org.bitcoinppl.cove.cloudbackup.CloudBackupEnableOnboardingView
import org.bitcoinppl.cove.cloudbackup.CloudBackupLastOutcomeNote
import org.bitcoinppl.cove.cloudbackup.CloudBackupManager
import org.bitcoinppl.cove.cloudbackup.configured
import org.bitcoinppl.cove.cloudbackup.draws
import org.bitcoinppl.cove.cloudbackup.isAvailable
import org.bitcoinppl.cove.cloudbackup.lookup
import org.bitcoinppl.cove.cloudbackup.section
import org.bitcoinppl.cove.cloudbackup.verification
import org.bitcoinppl.cove.ui.theme.CoveColor
import org.bitcoinppl.cove_core.CloudBackupActionAvailability
import org.bitcoinppl.cove_core.CloudBackupEnableContext
import org.bitcoinppl.cove_core.CloudBackupManagerAction
import org.bitcoinppl.cove_core.CloudBackupOnboardingCompletionReadiness
import org.bitcoinppl.cove_core.CloudBackupOutcomeTarget
import org.bitcoinppl.cove_core.CloudBackupPasskeyState
import org.bitcoinppl.cove_core.CloudBackupPendingEnableRecovery
import org.bitcoinppl.cove_core.CloudBackupSavedPasskeyLifecycle
import org.bitcoinppl.cove_core.CloudBackupScreen
import org.bitcoinppl.cove_core.CloudBackupScreenAction
import org.bitcoinppl.cove_core.CloudBackupScreenBody
import org.bitcoinppl.cove_core.CloudBackupSection
import org.bitcoinppl.cove_core.CloudBackupVerificationPresentation
import org.bitcoinppl.cove_core.CloudBackupVerificationSource
import org.bitcoinppl.cove_core.CloudBackupVerificationState
import org.bitcoinppl.cove_core.OnboardingBranch

internal fun shouldCompleteOnboardingCloudBackupFromPersistedState(
    readiness: CloudBackupOnboardingCompletionReadiness,
): Boolean = readiness == CloudBackupOnboardingCompletionReadiness.READY

internal fun isOnboardingCloudBackupEnableCompletion(context: CloudBackupEnableContext): Boolean =
    context.verificationSource == CloudBackupVerificationSource.ONBOARDING

internal sealed interface OnboardingCloudBackupStepPresentation {
    data object Enable : OnboardingCloudBackupStepPresentation

    data class PendingEnableRecovery(
        val recovery: CloudBackupPendingEnableRecovery,
        val cleanup: CloudBackupActionAvailability?,
    ) : OnboardingCloudBackupStepPresentation
}

internal enum class OnboardingCloudBackupRecoveryIntent {
    REMOVE_INCOMPLETE_SETUP,
    SKIP,
}

internal data class OnboardingCloudBackupStepActions(
    val onEnable: () -> Unit,
    val onEnabled: () -> Unit,
    val onSkip: () -> Unit,
    val dispatch: (CloudBackupManagerAction) -> Unit,
)

internal fun onboardingCloudBackupStepPresentation(
    screen: CloudBackupScreen,
): OnboardingCloudBackupStepPresentation =
    when (val body = screen.body) {
        is CloudBackupScreenBody.PendingEnableRecovery ->
            OnboardingCloudBackupStepPresentation.PendingEnableRecovery(
                recovery = body.v1,
                cleanup = screen.actions.lookup(CloudBackupScreenAction.CONFIRM_PENDING_ENABLE_CLEANUP),
            )
        else -> OnboardingCloudBackupStepPresentation.Enable
    }

internal fun routeOnboardingCloudBackupRecoveryIntent(
    intent: OnboardingCloudBackupRecoveryIntent,
    dispatch: (CloudBackupManagerAction) -> Unit,
    onSkip: () -> Unit,
) {
    when (intent) {
        OnboardingCloudBackupRecoveryIntent.REMOVE_INCOMPLETE_SETUP ->
            dispatch(CloudBackupManagerAction.ConfirmPendingEnableCleanup)
        OnboardingCloudBackupRecoveryIntent.SKIP -> onSkip()
    }
}

@Composable
internal fun OnboardingCloudBackupStepView(
    branch: OnboardingBranch?,
    onEnable: () -> Unit,
    onEnabled: () -> Unit,
    onSkip: () -> Unit,
) {
    val backupManager = remember { CloudBackupManager.getInstance() }

    OnboardingCloudBackupStepContent(
        backupManager = backupManager,
        branch = branch,
        actions =
            OnboardingCloudBackupStepActions(
                onEnable = onEnable,
                onEnabled = onEnabled,
                onSkip = onSkip,
                dispatch = backupManager::dispatch,
            ),
    )
}

@Composable
internal fun OnboardingCloudBackupStepContent(
    backupManager: CloudBackupManager,
    branch: OnboardingBranch?,
    actions: OnboardingCloudBackupStepActions,
) {
    ReportOnboardingCloudBackupEnabled(
        backupManager = backupManager,
        onEnabled = actions.onEnabled,
    )

    OnboardingCloudBackupPresentationContent(
        presentation = onboardingCloudBackupStepPresentation(backupManager.screen),
        branch = branch,
        actions = actions,
    )
}

@Composable
internal fun OnboardingCloudBackupEnableContent(
    branch: OnboardingBranch?,
    onEnable: () -> Unit,
    onSkip: () -> Unit,
) {
    when (branch) {
        OnboardingBranch.SOFTWARE_IMPORT -> {
            OnboardingSoftwareImportCloudBackupStepView(
                onEnable = onEnable,
                onSkip = onSkip,
            )
        }
        OnboardingBranch.HARDWARE -> {
            OnboardingHardwareImportCloudBackupStepView(
                onEnable = onEnable,
                onSkip = onSkip,
            )
        }
        else -> {
            OnboardingCloudBackupDetailsStepView(
                onEnable = onEnable,
                onSkip = onSkip,
                context = CloudBackupEnableOnboardingContext.STANDARD,
            )
        }
    }
}

@Composable
private fun OnboardingSoftwareImportCloudBackupStepView(
    onEnable: () -> Unit,
    onSkip: () -> Unit,
) {
    var showingDetails by rememberSaveable { mutableStateOf(false) }

    if (showingDetails) {
        OnboardingCloudBackupDetailsStepView(
            onEnable = onEnable,
            onSkip = { showingDetails = false },
            context = CloudBackupEnableOnboardingContext.STANDARD,
        )
    } else {
        OnboardingPromptScreen(
            icon = Icons.Default.CloudDownload,
            title = "Protect this wallet with Cloud Backup?",
            subtitle = "Cloud Backup makes it easier to recover this wallet if you lose this device.",
        ) {
            Surface(
                shape = RoundedCornerShape(18.dp),
                color = OnboardingCardFill,
                tonalElevation = 0.dp,
                shadowElevation = 0.dp,
                border = androidx.compose.foundation.BorderStroke(1.dp, OnboardingCardBorder),
            ) {
                Column(
                    modifier = Modifier.fillMaxWidth().padding(18.dp),
                    verticalArrangement = androidx.compose.foundation.layout.Arrangement.spacedBy(14.dp),
                ) {
                    Text(
                        text = "Your wallet backup is end-to-end encrypted before it leaves your device, stored in Google Drive, and locked with a passkey only you control.",
                        color = OnboardingTextSecondary,
                        style = MaterialTheme.typography.bodySmall.copy(lineHeight = 18.sp),
                    )
                    Text(
                        text = "You can skip this now and enable it later from Settings.",
                        color = CoveColor.coveLightGray.copy(alpha = 0.64f),
                        style = MaterialTheme.typography.bodySmall.copy(lineHeight = 18.sp),
                    )
                }
            }

            Spacer(modifier = Modifier.size(14.dp))
            OnboardingPrimaryButton(
                text = "Enable Cloud Backup",
                onClick = { showingDetails = true },
                modifier = Modifier.testTag("onboarding.cloudBackup.enable"),
            )
            Spacer(modifier = Modifier.size(14.dp))
            OnboardingSecondaryButton(
                text = "Not Now",
                onClick = onSkip,
                modifier = Modifier.testTag("onboarding.cloudBackup.skip"),
            )
        }
    }
}

@Composable
private fun OnboardingHardwareImportCloudBackupStepView(
    onEnable: () -> Unit,
    onSkip: () -> Unit,
) {
    var showingDetails by rememberSaveable { mutableStateOf(false) }

    if (showingDetails) {
        OnboardingCloudBackupDetailsStepView(
            onEnable = onEnable,
            onSkip = { showingDetails = false },
            context = CloudBackupEnableOnboardingContext.HARDWARE_IMPORT,
        )
    } else {
        OnboardingPromptScreen(
            icon = Icons.Default.CloudDownload,
            title = "Protect this hardware wallet with Cloud Backup?",
            subtitle = "Cloud Backup makes it easier to restore this wallet's configuration and labels if you lose this device.",
        ) {
            Surface(
                shape = RoundedCornerShape(18.dp),
                color = OnboardingCardFill,
                tonalElevation = 0.dp,
                shadowElevation = 0.dp,
                border = androidx.compose.foundation.BorderStroke(1.dp, OnboardingCardBorder),
            ) {
                Column(
                    modifier = Modifier.fillMaxWidth().padding(18.dp),
                    verticalArrangement = androidx.compose.foundation.layout.Arrangement.spacedBy(14.dp),
                ) {
                    Text(
                        text = "This backs up the imported hardware wallet configuration and labels stored in Cove so you can restore this wallet view later.",
                        color = OnboardingTextSecondary,
                        style = MaterialTheme.typography.bodySmall.copy(lineHeight = 18.sp),
                    )
                    Text(
                        text = "Enabling this also turns on Cloud Backup for Cove more broadly, so compatible wallets you create later, as well as wallet labels, will be backed up.",
                        color = OnboardingTextSecondary,
                        style = MaterialTheme.typography.bodySmall.copy(lineHeight = 18.sp),
                    )
                    Text(
                        text = "This does not back up your hardware wallet seed or private keys.",
                        color = Color.White.copy(alpha = 0.86f),
                        style = MaterialTheme.typography.bodySmall.copy(lineHeight = 18.sp),
                        fontWeight = FontWeight.SemiBold,
                    )
                    Text(
                        text = "You can skip this now and enable it later from Settings.",
                        color = CoveColor.coveLightGray.copy(alpha = 0.64f),
                        style = MaterialTheme.typography.bodySmall.copy(lineHeight = 18.sp),
                    )
                }
            }

            Spacer(modifier = Modifier.size(14.dp))
            OnboardingPrimaryButton(
                text = "Enable Cloud Backup",
                onClick = { showingDetails = true },
                modifier = Modifier.testTag("onboarding.cloudBackup.enable"),
            )
            Spacer(modifier = Modifier.size(14.dp))
            OnboardingSecondaryButton(
                text = "Not Now",
                onClick = onSkip,
                modifier = Modifier.testTag("onboarding.cloudBackup.skip"),
            )
        }
    }
}

@Composable
private fun OnboardingCloudBackupDetailsStepView(
    onEnable: () -> Unit,
    onSkip: () -> Unit,
    context: CloudBackupEnableOnboardingContext,
) {
    val backupManager = remember { CloudBackupManager.getInstance() }
    val screen = backupManager.screen
    val verification = screen.verification
    val verificationState = verification?.state
    val verificationFailed = verificationState is CloudBackupVerificationState.Failed
    val passkey = screen.configured?.section<CloudBackupSection.PasskeyCard>()?.v1
    val onboardingMessage =
        when {
            passkey is CloudBackupPasskeyState.UnsupportedProvider ->
                "This passkey provider did not confirm support for Cloud Backup. Try another supported provider such as 1Password or Bitwarden."
            screen.body is CloudBackupScreenBody.LoadFailed -> "Cove couldn't load Cloud Backup."
            verificationState is CloudBackupVerificationState.Failed -> verificationState.v1.message()
            else -> null
        }
    val enableFlow = (screen.body as? CloudBackupScreenBody.Enabling)?.v1

    // an Enable paused on its saved passkey owns the body while nothing runs, so this step keeps
    // drawing and offers Confirm Passkey as its primary button
    val needsManualPasskeyConfirmation =
        (screen.body as? CloudBackupScreenBody.SavedPasskeyConfirmation)?.v1?.lifecycle ==
            CloudBackupSavedPasskeyLifecycle.ENABLE

    // Try Again verifies, Confirm Passkey continues a paused enable, otherwise the button enables
    val primaryAction =
        when {
            verificationFailed -> CloudBackupScreenAction.VERIFY
            needsManualPasskeyConfirmation -> CloudBackupScreenAction.CONFIRM_SAVED_PASSKEY
            else -> CloudBackupScreenAction.ENABLE
        }
    val primary = screen.actions.lookup(primaryAction)
    val primaryButtonTitle =
        when (primaryAction) {
            CloudBackupScreenAction.VERIFY -> "Try Again"
            CloudBackupScreenAction.CONFIRM_SAVED_PASSKEY -> "Confirm Passkey"
            else -> "Enable Cloud Backup"
        }

    // the root prompt is answering the enable, so nothing covers it
    val isPromptingForEnableChoice = screen.actions.draws(CloudBackupScreenAction.ACCEPT_ENABLE_PROMPT_USE_EXISTING)
    val showsBusyOverlay =
        !isPromptingForEnableChoice &&
            (
                enableFlow != null ||
                    verificationState is CloudBackupVerificationState.Running ||
                    verificationState is CloudBackupVerificationState.AwaitingUploadConfirmation
            )

    fun cancelCloudBackupDetails() {
        if (screen.actions.lookup(CloudBackupScreenAction.DISCARD_PENDING_ENABLE)?.isAvailable == true) {
            backupManager.dispatch(CloudBackupManagerAction.DiscardPendingEnableCloudBackup)
        }

        onSkip()
    }

    // back stays on this step while a modal step covers it, the same as the hidden Back button
    BackHandler(enabled = showsBusyOverlay || isPromptingForEnableChoice) {}
    BackHandler(enabled = !showsBusyOverlay && !isPromptingForEnableChoice) {
        cancelCloudBackupDetails()
    }

    Box(modifier = Modifier.fillMaxSize()) {
        CloudBackupEnableOnboardingView(
            onEnable = {
                when (primaryAction) {
                    CloudBackupScreenAction.CONFIRM_SAVED_PASSKEY ->
                        backupManager.dispatch(CloudBackupManagerAction.ConfirmSavedPasskey)
                    CloudBackupScreenAction.VERIFY ->
                        backupManager.dispatch(
                            CloudBackupManagerAction.StartVerification(
                                CloudBackupVerificationSource.ONBOARDING,
                            ),
                        )
                    else -> onEnable()
                }
            },
            onCancel = { cancelCloudBackupDetails() },
            message = onboardingMessage,
            primaryAvailability = primary,
            context = context,
            primaryButtonTitle = primaryButtonTitle,
            // onboarding's own identifier, which its flow tests share with the screens before this one
            primaryTestTag = "onboarding.cloudBackup.enable",
            cancelButtonTitle = "Back",
            cancelButtonLeading = true,
            cancelEnabled = !showsBusyOverlay && !isPromptingForEnableChoice,
            footer = {
                CloudBackupLastOutcomeNote(
                    outcome = screen.lastOutcome,
                    target = CloudBackupOutcomeTarget.Action(primaryAction),
                    onDispatch = backupManager::dispatch,
                )
            },
        )

        if (showsBusyOverlay) {
            CloudBackupEnableBusyOverlay(
                enableFlow,
                verification?.presentation ?: CloudBackupVerificationPresentation.Hidden(null),
            )
        }
    }
}

@Composable
internal fun OnboardingCloudBackupSuccessView(
    onContinue: () -> Unit,
) {
    OnboardingBackground {
        Column(
            modifier =
                Modifier
                    .fillMaxSize()
                    .navigationBarsPadding()
                    .padding(horizontal = 24.dp, vertical = 28.dp),
            horizontalAlignment = Alignment.CenterHorizontally,
        ) {
            Spacer(modifier = Modifier.weight(1f, fill = true))

            OnboardingStatusHero(
                icon = Icons.Default.Check,
                tint = OnboardingSuccess,
                fillColor = OnboardingSuccess.copy(alpha = 0.12f),
            )

            Spacer(modifier = Modifier.height(36.dp))

            Text(
                text = "Cloud Backup enabled",
                color = Color.White,
                style = MaterialTheme.typography.headlineSmall,
                fontWeight = FontWeight.SemiBold,
                textAlign = TextAlign.Center,
                modifier = Modifier.fillMaxWidth(),
            )

            Spacer(modifier = Modifier.height(12.dp))

            Text(
                text = "Cove can finish confirming cloud visibility in the background.",
                color = OnboardingTextSecondary,
                style = MaterialTheme.typography.bodyLarge,
                textAlign = TextAlign.Center,
                modifier = Modifier.fillMaxWidth(),
            )

            Spacer(modifier = Modifier.weight(1f, fill = true))

            OnboardingPrimaryButton(
                text = "Continue",
                onClick = onContinue,
                modifier = Modifier.testTag("onboarding.cloudBackup.success.continue"),
            )
        }
    }
}
