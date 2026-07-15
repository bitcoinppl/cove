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
import org.bitcoinppl.cove.cloudbackup.CloudBackupManager
import org.bitcoinppl.cove.ui.theme.CoveColor
import org.bitcoinppl.cove_core.CloudBackupEnableContext
import org.bitcoinppl.cove_core.CloudBackupEnableFlow
import org.bitcoinppl.cove_core.CloudBackupManagerAction
import org.bitcoinppl.cove_core.CloudBackupOnboardingCompletionReadiness
import org.bitcoinppl.cove_core.CloudBackupPasskeyChoiceIntent
import org.bitcoinppl.cove_core.CloudBackupPendingEnableRecovery
import org.bitcoinppl.cove_core.CloudBackupRootPrompt
import org.bitcoinppl.cove_core.CloudBackupLifecycle
import org.bitcoinppl.cove_core.CloudBackupVerificationSource
import org.bitcoinppl.cove_core.CloudBackupVerificationState
import org.bitcoinppl.cove_core.OnboardingBranch
import org.bitcoinppl.cove_core.SavedPasskeyConfirmationMode

internal fun shouldCompleteOnboardingCloudBackupFromPersistedState(
    readiness: CloudBackupOnboardingCompletionReadiness,
): Boolean = readiness == CloudBackupOnboardingCompletionReadiness.READY

internal fun isOnboardingCloudBackupEnableCompletion(context: CloudBackupEnableContext): Boolean =
    context.verificationSource == CloudBackupVerificationSource.ONBOARDING

internal sealed interface OnboardingCloudBackupStepPresentation {
    data object Enable : OnboardingCloudBackupStepPresentation

    data class PendingEnableRecovery(
        val recovery: CloudBackupPendingEnableRecovery,
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
    lifecycle: CloudBackupLifecycle,
): OnboardingCloudBackupStepPresentation =
    when (lifecycle) {
        is CloudBackupLifecycle.PendingEnableRecovery ->
            OnboardingCloudBackupStepPresentation.PendingEnableRecovery(lifecycle.v1)
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
        presentation = onboardingCloudBackupStepPresentation(backupManager.lifecycle),
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

    val lifecycleMsg = backupManager.lifecycleFailureMessage
    val onboardingMessage =
        when {
            backupManager.isUnsupportedPasskeyProvider ->
                "This passkey provider did not confirm support for Cloud Backup. Try another supported provider such as 1Password or Bitwarden."
            lifecycleMsg != null -> lifecycleMsg
            else -> (backupManager.verificationState as? CloudBackupVerificationState.Failed)?.v1?.message()
        }
    val isVerifying = backupManager.verificationState is CloudBackupVerificationState.Running
    val verificationFailed = backupManager.verificationState is CloudBackupVerificationState.Failed
    val isConfirmingUpload = backupManager.hasPendingUploadVerification
    val savedPasskeyConfirmationMode =
        (backupManager.enableFlow as? CloudBackupEnableFlow.AwaitingSavedPasskeyConfirmation)?.v1
    val needsManualPasskeyConfirmation =
        savedPasskeyConfirmationMode == SavedPasskeyConfirmationMode.MANUAL
    val isEnabling = backupManager.isLifecycleEnabling && !needsManualPasskeyConfirmation
    val isBusy =
        isVerifying ||
            isConfirmingUpload ||
            backupManager.enableFlow == CloudBackupEnableFlow.ConfirmingSavedPasskey ||
            isEnabling
    val primaryButtonTitle =
        when {
            verificationFailed -> "Try Again"
            needsManualPasskeyConfirmation -> "Confirm Passkey"
            else -> null
        }
            ?: "Enable Cloud Backup"
    val rootPrompt = backupManager.rootPrompt
    val isPromptingForEnableChoice =
        rootPrompt is CloudBackupRootPrompt.ExistingBackupFound ||
            (
                rootPrompt is CloudBackupRootPrompt.PasskeyChoice &&
                    (
                        rootPrompt.v1 is CloudBackupPasskeyChoiceIntent.Enable ||
                            rootPrompt.v1 is CloudBackupPasskeyChoiceIntent.EnableExistingPasskeyOnly
                    )
            )

    fun cancelCloudBackupDetails() {
        if (needsManualPasskeyConfirmation) {
            backupManager.dispatch(CloudBackupManagerAction.DiscardPendingEnableCloudBackup)
        }

        onSkip()
    }

    BackHandler {
        if (isBusy || isPromptingForEnableChoice) {
            return@BackHandler
        }

        cancelCloudBackupDetails()
    }

    Box(modifier = Modifier.fillMaxSize()) {
        CloudBackupEnableOnboardingView(
            onEnable = {
                if (isBusy || isPromptingForEnableChoice) {
                    return@CloudBackupEnableOnboardingView
                }

                if (needsManualPasskeyConfirmation) {
                    backupManager.dispatch(CloudBackupManagerAction.ConfirmSavedPasskey)
                    return@CloudBackupEnableOnboardingView
                }

                if (verificationFailed) {
                    backupManager.dispatch(
                        CloudBackupManagerAction.StartVerification(
                            CloudBackupVerificationSource.ONBOARDING,
                        ),
                    )
                    return@CloudBackupEnableOnboardingView
                }

                onEnable()
            },
            onCancel = { cancelCloudBackupDetails() },
            message = onboardingMessage,
            isBusy = isBusy || isPromptingForEnableChoice,
            context = context,
            primaryButtonTitle = primaryButtonTitle,
            cancelButtonTitle = "Back",
            cancelButtonLeading = true,
        )

        if (isBusy) {
            CloudBackupEnableBusyOverlay(
                backupManager.enableFlow,
                backupManager.verificationPresentation,
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
