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
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import org.bitcoinppl.cove.R
import org.bitcoinppl.cove.cloudbackup.CloudBackupEnableBusyOverlay
import org.bitcoinppl.cove.cloudbackup.CloudBackupEnableOnboardingContext
import org.bitcoinppl.cove.cloudbackup.CloudBackupEnableOnboardingView
import org.bitcoinppl.cove.cloudbackup.CloudBackupManager
import org.bitcoinppl.cove.localizedMessage
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
            title = stringResource(R.string.onboarding_cloud_backup_software_title),
            subtitle = stringResource(R.string.onboarding_cloud_backup_software_subtitle),
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
                        text = stringResource(R.string.onboarding_cloud_backup_software_body),
                        color = OnboardingTextSecondary,
                        style = MaterialTheme.typography.bodySmall.copy(lineHeight = 18.sp),
                    )
                    Text(
                        text = stringResource(R.string.onboarding_cloud_backup_skip_later),
                        color = CoveColor.coveLightGray.copy(alpha = 0.64f),
                        style = MaterialTheme.typography.bodySmall.copy(lineHeight = 18.sp),
                    )
                }
            }

            Spacer(modifier = Modifier.size(14.dp))
            OnboardingPrimaryButton(
                text = stringResource(R.string.onboarding_enable_cloud_backup),
                onClick = { showingDetails = true },
                modifier = Modifier.testTag("onboarding.cloudBackup.enable"),
            )
            Spacer(modifier = Modifier.size(14.dp))
            OnboardingSecondaryButton(
                text = stringResource(R.string.onboarding_not_now),
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
            title = stringResource(R.string.onboarding_cloud_backup_hardware_title),
            subtitle = stringResource(R.string.onboarding_cloud_backup_hardware_subtitle),
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
                        text = stringResource(R.string.onboarding_cloud_backup_hardware_body),
                        color = OnboardingTextSecondary,
                        style = MaterialTheme.typography.bodySmall.copy(lineHeight = 18.sp),
                    )
                    Text(
                        text = stringResource(R.string.onboarding_cloud_backup_hardware_broader_body),
                        color = OnboardingTextSecondary,
                        style = MaterialTheme.typography.bodySmall.copy(lineHeight = 18.sp),
                    )
                    Text(
                        text = stringResource(R.string.onboarding_cloud_backup_hardware_seed_notice),
                        color = Color.White.copy(alpha = 0.86f),
                        style = MaterialTheme.typography.bodySmall.copy(lineHeight = 18.sp),
                        fontWeight = FontWeight.SemiBold,
                    )
                    Text(
                        text = stringResource(R.string.onboarding_cloud_backup_skip_later),
                        color = CoveColor.coveLightGray.copy(alpha = 0.64f),
                        style = MaterialTheme.typography.bodySmall.copy(lineHeight = 18.sp),
                    )
                }
            }

            Spacer(modifier = Modifier.size(14.dp))
            OnboardingPrimaryButton(
                text = stringResource(R.string.onboarding_enable_cloud_backup),
                onClick = { showingDetails = true },
                modifier = Modifier.testTag("onboarding.cloudBackup.enable"),
            )
            Spacer(modifier = Modifier.size(14.dp))
            OnboardingSecondaryButton(
                text = stringResource(R.string.onboarding_not_now),
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

    val lifecycleMsg = backupManager.lifecycleFailure?.localizedMessage()?.asString()
    val onboardingMessage =
        when {
            backupManager.isUnsupportedPasskeyProvider ->
                stringResource(R.string.onboarding_unsupported_passkey_provider)
            lifecycleMsg != null -> lifecycleMsg
            else -> (backupManager.verificationState as? CloudBackupVerificationState.Failed)?.v1?.localizedMessage()?.asString()
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
            verificationFailed -> stringResource(R.string.scoped_common_try_again)
            needsManualPasskeyConfirmation -> stringResource(R.string.onboarding_confirm_passkey)
            else -> null
        }
            ?: stringResource(R.string.onboarding_enable_cloud_backup)
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
            cancelButtonTitle = stringResource(R.string.scoped_common_back),
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
                text = stringResource(R.string.onboarding_cloud_backup_success),
                color = Color.White,
                style = MaterialTheme.typography.headlineSmall,
                fontWeight = FontWeight.SemiBold,
                textAlign = TextAlign.Center,
                modifier = Modifier.fillMaxWidth(),
            )

            Spacer(modifier = Modifier.weight(1f, fill = true))

            OnboardingPrimaryButton(
                text = stringResource(R.string.scoped_common_continue),
                onClick = onContinue,
                modifier = Modifier.testTag("onboarding.cloudBackup.success.continue"),
            )
        }
    }
}
