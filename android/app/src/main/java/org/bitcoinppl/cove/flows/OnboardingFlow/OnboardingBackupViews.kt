package org.bitcoinppl.cove.flows.OnboardingFlow

import android.content.ClipData
import android.content.ClipboardManager
import android.content.Context
import androidx.activity.compose.BackHandler
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.aspectRatio
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Check
import androidx.compose.material.icons.filled.CloudDownload
import androidx.compose.material.icons.filled.Description
import androidx.compose.material.icons.filled.Lock
import androidx.compose.material.icons.filled.Warning
import androidx.compose.material.icons.filled.Wallet
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.google.zxing.qrcode.decoder.ErrorCorrectionLevel
import kotlinx.coroutines.delay
import org.bitcoinppl.cove.AppManager
import org.bitcoinppl.cove.Log
import org.bitcoinppl.cove.OnboardingManager
import org.bitcoinppl.cove.QrCodeGenerator
import org.bitcoinppl.cove.cloudbackup.CloudBackupEnableBusyOverlay
import org.bitcoinppl.cove.cloudbackup.CloudBackupEnableOnboardingContext
import org.bitcoinppl.cove.cloudbackup.CloudBackupEnableOnboardingView
import org.bitcoinppl.cove.cloudbackup.CloudBackupManager
import org.bitcoinppl.cove.cloudbackup.CloudBackupPendingEnableRecoveryContent
import org.bitcoinppl.cove.ui.theme.CoveColor
import org.bitcoinppl.cove.ui.theme.caption
import org.bitcoinppl.cove_core.CloudBackupConfiguredState
import org.bitcoinppl.cove_core.CloudBackupEnableFlow
import org.bitcoinppl.cove_core.CloudBackupLifecycle
import org.bitcoinppl.cove_core.CloudBackupManagerAction
import org.bitcoinppl.cove_core.CloudBackupPasskeyChoiceIntent
import org.bitcoinppl.cove_core.CloudBackupPasskeyState
import org.bitcoinppl.cove_core.CloudBackupRestoreFlow
import org.bitcoinppl.cove_core.CloudBackupRootPrompt
import org.bitcoinppl.cove_core.CloudBackupVerificationSource
import org.bitcoinppl.cove_core.CloudBackupVerificationState
import org.bitcoinppl.cove_core.OnboardingAction
import org.bitcoinppl.cove_core.OnboardingBranch
import org.bitcoinppl.cove_core.OnboardingRestoreState
import org.bitcoinppl.cove_core.SavedPasskeyConfirmationMode

@Composable
internal fun OnboardingCreatingWalletView(
    onContinue: () -> Unit,
) {
    var didAdvance by remember { mutableStateOf(false) }

    OnboardingBackground {
        Column(
            modifier =
                Modifier
                    .fillMaxSize()
                    .padding(horizontal = 28.dp, vertical = 18.dp),
            horizontalAlignment = Alignment.CenterHorizontally,
            verticalArrangement = Arrangement.Center,
        ) {
            OnboardingStatusHero(
                icon = Icons.Default.Wallet,
                pulse = true,
            )

            Spacer(modifier = Modifier.size(40.dp))

            Text(
                text = "Creating your wallet",
                color = Color.White,
                style = MaterialTheme.typography.headlineSmall,
                fontWeight = FontWeight.SemiBold,
                textAlign = TextAlign.Center,
            )

            Spacer(modifier = Modifier.size(12.dp))

            Text(
                text = "Generating keys and preparing your backup flow",
                color = OnboardingTextSecondary,
                style = MaterialTheme.typography.bodyMedium,
                textAlign = TextAlign.Center,
            )

            Spacer(modifier = Modifier.size(16.dp))

            CircularProgressIndicator(color = Color.White)
        }
    }

    LaunchedEffect(Unit) {
        if (didAdvance) return@LaunchedEffect
        delay(900)
        if (didAdvance) return@LaunchedEffect
        didAdvance = true
        onContinue()
    }
}

@Composable
internal fun OnboardingBackupWalletView(
    branch: OnboardingBranch?,
    secretWordsSaved: Boolean,
    cloudBackupEnabled: Boolean,
    wordCount: Int,
    onShowWords: () -> Unit,
    onEnableCloudBackup: () -> Unit,
    onContinue: () -> Unit,
) {
    val title =
        if (branch == OnboardingBranch.EXCHANGE) {
            "Back up your wallet before funding it"
        } else {
            "Back up your wallet"
        }
    val subtitle =
        if (branch == OnboardingBranch.EXCHANGE) {
            "You'll fund this wallet next. Save your recovery words or enable Cloud Backup first."
        } else {
            "Choose at least one backup method before continuing."
        }

    OnboardingPromptScreen(
        icon = Icons.Default.Lock,
        title = title,
        subtitle = subtitle,
    ) {
        Column(verticalArrangement = Arrangement.spacedBy(14.dp)) {
            OnboardingStatusCard(
                title = "Save recovery words",
                subtitle = "Write down your $wordCount-word recovery phrase offline",
                actionTitle = if (secretWordsSaved) "Saved" else "Show Words",
                icon = Icons.Default.Description,
                isComplete = secretWordsSaved,
                onClick = onShowWords,
                modifier = Modifier.testTag("onboarding.secretWords"),
            )
            OnboardingStatusCard(
                title = "Enable Cloud Backup",
                subtitle = "Encrypt and store a backup in Google Drive protected by your passkey",
                actionTitle = if (cloudBackupEnabled) "Enabled" else "Enable",
                icon = Icons.Default.CloudDownload,
                isComplete = cloudBackupEnabled,
                onClick = onEnableCloudBackup,
                modifier = Modifier.testTag("onboarding.cloudBackup.prompt"),
            )
        }

        Spacer(modifier = Modifier.size(16.dp))

        OnboardingPrimaryButton(
            text = "Continue",
            onClick = onContinue,
            modifier = Modifier.testTag("onboarding.continue"),
            enabled = secretWordsSaved || cloudBackupEnabled,
        )
    }
}

@Composable
internal fun ReportOnboardingCloudBackupEnabled(
    backupManager: CloudBackupManager,
    onEnabled: () -> Unit,
) {
    var didReportEnabled by remember { mutableStateOf(false) }

    LaunchedEffect(backupManager.enableCompletion?.id, backupManager.lifecycle) {
        if (didReportEnabled) return@LaunchedEffect

        val completion = backupManager.enableCompletion
        if (completion != null && isOnboardingCloudBackupEnableCompletion(completion.item)) {
            backupManager.consumeEnableCompletion(completion)
            didReportEnabled = true
            onEnabled()
            return@LaunchedEffect
        }

        val readiness = backupManager.onboardingEnableCompletionReadiness()
        if (
            didReportEnabled ||
            !shouldCompleteOnboardingCloudBackupFromPersistedState(readiness)
        ) {
            return@LaunchedEffect
        }

        didReportEnabled = true
        onEnabled()
    }
}

@Composable
internal fun OnboardingCloudBackupPresentationContent(
    presentation: OnboardingCloudBackupStepPresentation,
    branch: OnboardingBranch?,
    actions: OnboardingCloudBackupStepActions,
) {
    when (presentation) {
        is OnboardingCloudBackupStepPresentation.PendingEnableRecovery -> {
            CloudBackupPendingEnableRecoveryContent(
                recovery = presentation.recovery,
                onConfirmCleanup = {
                    routeOnboardingCloudBackupRecoveryIntent(
                        OnboardingCloudBackupRecoveryIntent.REMOVE_INCOMPLETE_SETUP,
                        dispatch = actions.dispatch,
                        onSkip = actions.onSkip,
                    )
                },
                onCancel = {
                    routeOnboardingCloudBackupRecoveryIntent(
                        OnboardingCloudBackupRecoveryIntent.SKIP,
                        dispatch = actions.dispatch,
                        onSkip = actions.onSkip,
                    )
                },
            )
        }

        OnboardingCloudBackupStepPresentation.Enable -> {
            OnboardingCloudBackupEnableContent(
                branch = branch,
                onEnable = actions.onEnable,
                onSkip = actions.onSkip,
            )
        }
    }
}

