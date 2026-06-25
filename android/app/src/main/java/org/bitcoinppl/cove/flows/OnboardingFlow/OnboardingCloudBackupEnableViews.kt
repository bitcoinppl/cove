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
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
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
import org.bitcoinppl.cove_core.CloudBackupConfiguredState
import org.bitcoinppl.cove_core.CloudBackupEnableFlow
import org.bitcoinppl.cove_core.CloudBackupManagerAction
import org.bitcoinppl.cove_core.CloudBackupPasskeyChoiceIntent
import org.bitcoinppl.cove_core.CloudBackupPasskeyState
import org.bitcoinppl.cove_core.CloudBackupRootPrompt
import org.bitcoinppl.cove_core.CloudBackupVerificationSource
import org.bitcoinppl.cove_core.CloudBackupVerificationState
import org.bitcoinppl.cove_core.OnboardingBranch
import org.bitcoinppl.cove_core.SavedPasskeyConfirmationMode

internal fun shouldCompleteOnboardingCloudBackup(
    configuredState: CloudBackupConfiguredState?,
    hasPendingUploadVerification: Boolean,
): Boolean {
    configuredState ?: return false
    if (configuredState.passkey != CloudBackupPasskeyState.Available) return false
    if (hasPendingUploadVerification) return false

    val verification = configuredState.verification as? CloudBackupVerificationState.Verified
    return verification?.report != null
}

@Composable
internal fun OnboardingCloudBackupStepView(
    branch: OnboardingBranch?,
    onEnable: () -> Unit,
    onEnabled: () -> Unit,
    onSkip: () -> Unit,
) {
    when (branch) {
        OnboardingBranch.SOFTWARE_IMPORT -> {
            OnboardingSoftwareImportCloudBackupStepView(
                onEnable = onEnable,
                onEnabled = onEnabled,
                onSkip = onSkip,
            )
        }
        OnboardingBranch.HARDWARE -> {
            OnboardingHardwareImportCloudBackupStepView(
                onEnable = onEnable,
                onEnabled = onEnabled,
                onSkip = onSkip,
            )
        }
        else -> {
            OnboardingCloudBackupDetailsStepView(
                onEnable = onEnable,
                onEnabled = onEnabled,
                onSkip = onSkip,
                context = CloudBackupEnableOnboardingContext.STANDARD,
            )
        }
    }
}

@Composable
private fun OnboardingSoftwareImportCloudBackupStepView(
    onEnable: () -> Unit,
    onEnabled: () -> Unit,
    onSkip: () -> Unit,
) {
    var showingDetails by remember { mutableStateOf(false) }

    if (showingDetails) {
        OnboardingCloudBackupDetailsStepView(
            onEnable = onEnable,
            onEnabled = onEnabled,
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
    onEnabled: () -> Unit,
    onSkip: () -> Unit,
) {
    var showingDetails by remember { mutableStateOf(false) }

    if (showingDetails) {
        OnboardingCloudBackupDetailsStepView(
            onEnable = onEnable,
            onEnabled = onEnabled,
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
    onEnabled: () -> Unit,
    onSkip: () -> Unit,
    context: CloudBackupEnableOnboardingContext,
) {
    val backupManager = remember { CloudBackupManager.getInstance() }
    var didReportEnabled by remember { mutableStateOf(false) }

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
                    rootPrompt.v1 is CloudBackupPasskeyChoiceIntent.Enable
            )

    fun cancelCloudBackupDetails() {
        if (needsManualPasskeyConfirmation) {
            backupManager.dispatch(CloudBackupManagerAction.DiscardPendingEnableCloudBackup)
        }

        onSkip()
    }

    fun completeIfEnabled() {
        if (didReportEnabled) return
        if (!shouldCompleteOnboardingCloudBackup(
                backupManager.configuredState,
                backupManager.hasPendingUploadVerification,
            )
        ) {
            return
        }

        didReportEnabled = true
        onEnabled()
    }

    LaunchedEffect(
        backupManager.configuredState,
        backupManager.hasPendingUploadVerification,
    ) {
        completeIfEnabled()
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
            CloudBackupEnableBusyOverlay(backupManager.enableFlow)
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
                text = "Cloud Backup enabled successfully",
                color = Color.White,
                style = MaterialTheme.typography.headlineSmall,
                fontWeight = FontWeight.SemiBold,
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
