package org.bitcoinppl.cove.flows.OnboardingFlow

import android.content.ClipData
import android.content.ClipboardManager
import android.content.Context
import android.view.WindowManager
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
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
import androidx.compose.foundation.lazy.grid.GridCells
import androidx.compose.foundation.lazy.grid.LazyVerticalGrid
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
import androidx.compose.runtime.DisposableEffect
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
import org.bitcoinppl.cove.ScreenSecurity
import org.bitcoinppl.cove.cloudbackup.CloudBackupEnableBusyOverlay
import org.bitcoinppl.cove.cloudbackup.CloudBackupEnableOnboardingContext
import org.bitcoinppl.cove.cloudbackup.CloudBackupEnableOnboardingView
import org.bitcoinppl.cove.cloudbackup.CloudBackupManager
import org.bitcoinppl.cove.findActivity
import org.bitcoinppl.cove.ui.theme.CoveColor
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

internal fun combinedRestoreProgress(restoreState: OnboardingRestoreState): Float {
    val flow = (restoreState as? OnboardingRestoreState.Restoring)?.v1 ?: return 0f

    return when (flow) {
        CloudBackupRestoreFlow.Finding -> 0f
        is CloudBackupRestoreFlow.Downloading -> {
            val total = flow.total.toFloat()
            if (total <= 0f) return 0f
            flow.completed.toFloat() / (total * 2f)
        }
        is CloudBackupRestoreFlow.Restoring -> {
            val total = flow.total.toFloat()
            if (total <= 0f) return 0f
            (total + flow.completed.toFloat()) / (total * 2f)
        }
    }
}

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
internal fun OnboardingSecretWordsView(
    words: List<String>,
    onBack: () -> Unit,
    onSaved: () -> Unit,
) {
    val context = LocalContext.current

    DisposableEffect(Unit) {
        val window = context.findActivity()?.window
        ScreenSecurity.enter()
        window?.setFlags(
            WindowManager.LayoutParams.FLAG_SECURE,
            WindowManager.LayoutParams.FLAG_SECURE,
        )
        onDispose {
            ScreenSecurity.exit()
            if (!ScreenSecurity.isSensitiveScreen) {
                window?.clearFlags(WindowManager.LayoutParams.FLAG_SECURE)
            }
        }
    }

    OnboardingBackground {
        Column(modifier = Modifier.fillMaxSize()) {
            Row(
                modifier =
                    Modifier
                        .fillMaxWidth()
                        .statusBarsPadding()
                        .padding(horizontal = 24.dp)
                        .padding(top = 20.dp),
                horizontalArrangement = Arrangement.Start,
            ) {
                Text(
                    text = "Back",
                    color = Color.White,
                    style = MaterialTheme.typography.titleMedium,
                    fontWeight = FontWeight.SemiBold,
                    modifier = Modifier.clickable(onClick = onBack),
                )
            }

            Column(
                modifier =
                    Modifier
                        .weight(1f)
                        .verticalScroll(rememberScrollState())
                        .padding(horizontal = 24.dp)
                        .padding(top = 32.dp),
            ) {
                Text(
                    text = "Your Recovery Words",
                    color = Color.White,
                    fontSize = 34.sp,
                    lineHeight = 38.sp,
                    fontWeight = FontWeight.SemiBold,
                )

                Spacer(modifier = Modifier.size(12.dp))

                Text(
                    text = "Write these down exactly in order and keep them offline. Anyone with these words can control your Bitcoin.",
                    color = OnboardingTextSecondary,
                    style = MaterialTheme.typography.bodySmall.copy(lineHeight = 18.sp),
                )

                Spacer(modifier = Modifier.size(24.dp))

                LazyVerticalGrid(
                    columns = GridCells.Fixed(2),
                    verticalArrangement = Arrangement.spacedBy(12.dp),
                    horizontalArrangement = Arrangement.spacedBy(12.dp),
                    modifier = Modifier.fillMaxWidth().height(gridHeightForWordCount(words.size)),
                ) {
                    items(words.size) { index ->
                        OnboardingWordCard(
                            index = index + 1,
                            word = words[index],
                        )
                    }
                }

                Spacer(modifier = Modifier.size(24.dp))
            }

            Column(
                modifier =
                    Modifier
                        .fillMaxWidth()
                        .navigationBarsPadding()
                        .padding(horizontal = 24.dp)
                        .padding(top = 12.dp, bottom = 24.dp),
            ) {
                OnboardingPrimaryButton(
                    text = "I Saved These Words",
                    onClick = onSaved,
                    modifier = Modifier.testTag("onboarding.secretWords.saved"),
                )
            }
        }
    }
}

private fun gridHeightForWordCount(wordCount: Int) = (((wordCount + 1) / 2).coerceAtLeast(1) * 74).dp

@Composable
private fun OnboardingWordCard(
    index: Int,
    word: String,
) {
    Surface(
        shape = RoundedCornerShape(16.dp),
        color = OnboardingCardFill,
        tonalElevation = 0.dp,
        shadowElevation = 0.dp,
        border = androidx.compose.foundation.BorderStroke(1.dp, OnboardingCardBorder),
    ) {
        Row(
            modifier = Modifier.fillMaxWidth().padding(horizontal = 14.dp, vertical = 14.dp),
            horizontalArrangement = Arrangement.spacedBy(12.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Box(
                modifier =
                    Modifier
                        .size(26.dp)
                        .clip(RoundedCornerShape(99.dp))
                        .background(Color.White.copy(alpha = 0.08f)),
                contentAlignment = Alignment.Center,
            ) {
                Text(
                    text = index.toString(),
                    color = OnboardingGradientLight,
                    style = MaterialTheme.typography.labelSmall,
                    fontWeight = FontWeight.SemiBold,
                )
            }

            Text(
                text = word,
                color = Color.White,
                style = MaterialTheme.typography.bodyMedium,
                fontWeight = FontWeight.Medium,
            )
        }
    }
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
                    verticalArrangement = Arrangement.spacedBy(14.dp),
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
                    verticalArrangement = Arrangement.spacedBy(14.dp),
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
        rootPrompt is CloudBackupRootPrompt.PasskeyChoice &&
            rootPrompt.v1 is CloudBackupPasskeyChoiceIntent.Enable

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
            onCancel = {
                if (needsManualPasskeyConfirmation) {
                    backupManager.dispatch(CloudBackupManagerAction.DiscardPendingEnableCloudBackup)
                }

                onSkip()
            },
            message = onboardingMessage,
            isBusy = isBusy || isPromptingForEnableChoice,
            context = context,
            primaryButtonTitle = primaryButtonTitle,
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
                fontSize = 28.sp,
                lineHeight = 34.sp,
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

@Composable
internal fun OnboardingRestoreView(
    restoreState: OnboardingRestoreState,
    onDone: () -> Unit,
    onRetry: () -> Unit,
    onContinueWithoutBackup: () -> Unit,
) {
    OnboardingRestoreContent(
        restoreState = restoreState,
        combinedProgress = combinedRestoreProgress(restoreState),
        onDone = onDone,
        onRetry = onRetry,
        onContinueWithoutBackup = onContinueWithoutBackup,
    )
}

@Composable
private fun OnboardingRestoreContent(
    restoreState: OnboardingRestoreState,
    combinedProgress: Float,
    onDone: () -> Unit,
    onRetry: () -> Unit,
    onContinueWithoutBackup: () -> Unit,
) {
    OnboardingBackground {
        BoxWithConstraints(
            modifier =
                Modifier
                    .fillMaxSize(),
        ) {
            Column(
                modifier =
                    Modifier
                        .fillMaxWidth()
                        .heightIn(min = maxHeight)
                        .verticalScroll(rememberScrollState())
                        .padding(horizontal = 28.dp, vertical = 18.dp),
                horizontalAlignment = Alignment.CenterHorizontally,
                verticalArrangement = Arrangement.Center,
            ) {
                when (restoreState) {
                    OnboardingRestoreState.Idle,
                    is OnboardingRestoreState.Restoring,
                    -> OnboardingStatusHero(icon = Icons.Default.CloudDownload)
                    is OnboardingRestoreState.Complete ->
                        OnboardingStatusHero(
                            icon = Icons.Default.Check,
                            tint = OnboardingSuccess,
                            fillColor = OnboardingSuccess.copy(alpha = 0.12f),
                        )
                    is OnboardingRestoreState.Failed ->
                        OnboardingStatusHero(
                            icon = Icons.Default.Warning,
                            tint = Color.Red,
                            fillColor = Color.Red.copy(alpha = 0.12f),
                        )
                }

                Spacer(modifier = Modifier.size(44.dp))

                when (restoreState) {
                    OnboardingRestoreState.Idle,
                    is OnboardingRestoreState.Restoring,
                    -> {
                        Text(
                            text = "Restoring from Google Drive...",
                            color = Color.White,
                            style = MaterialTheme.typography.headlineSmall,
                            fontWeight = FontWeight.SemiBold,
                            textAlign = TextAlign.Center,
                        )
                        Spacer(modifier = Modifier.size(10.dp))
                        Text(
                            text = "This might take a few minutes",
                            color = OnboardingTextSecondary,
                            style = MaterialTheme.typography.bodyMedium,
                            textAlign = TextAlign.Center,
                        )
                        Spacer(modifier = Modifier.size(18.dp))
                        OnboardingThinProgressBar(progress = combinedProgress)
                    }
                    is OnboardingRestoreState.Complete -> {
                        val failedCount = restoreState.v1.walletsFailed.toInt()
                        Text(
                            text = if (failedCount == 0) "You're all set" else "Some wallets were restored",
                            color = Color.White,
                            style = MaterialTheme.typography.headlineSmall,
                            fontWeight = FontWeight.SemiBold,
                            textAlign = TextAlign.Center,
                        )
                        Spacer(modifier = Modifier.size(10.dp))
                        Text(
                            text =
                                if (failedCount == 0) {
                                    "Your wallets have been restored."
                                } else {
                                    "${pluralize(failedCount, "wallet", "wallets")} could not be restored. You can retry from backup settings."
                                },
                            color = OnboardingTextSecondary,
                            style = MaterialTheme.typography.bodyMedium,
                            textAlign = TextAlign.Center,
                        )
                    }
                    is OnboardingRestoreState.Failed -> {
                        Text(
                            text = "Restore Failed",
                            color = Color.White,
                            fontSize = 34.sp,
                            lineHeight = 38.sp,
                            fontWeight = FontWeight.Bold,
                            textAlign = TextAlign.Center,
                        )
                        Spacer(modifier = Modifier.size(12.dp))
                        Text(
                            text = "Something went wrong while restoring your wallets",
                            color = OnboardingTextSecondary,
                            style = MaterialTheme.typography.bodyMedium,
                            textAlign = TextAlign.Center,
                        )
                    }
                }

                Spacer(modifier = Modifier.size(28.dp))

                when (restoreState) {
                    OnboardingRestoreState.Idle,
                    is OnboardingRestoreState.Restoring,
                    -> Unit
                    is OnboardingRestoreState.Complete -> {
                        val report = restoreState.v1
                        if (report.walletsFailed.toInt() > 0) {
                            OnboardingInlineMessage(text = "${report.walletsFailed} wallet(s) could not be restored")
                            Spacer(modifier = Modifier.size(16.dp))
                        }
                        if (report.labelsFailedWalletNames.isNotEmpty()) {
                            OnboardingInlineMessage(
                                text = "${report.labelsFailedWalletNames.size} restored wallet(s) had labels that could not be imported",
                            )
                            Spacer(modifier = Modifier.size(16.dp))
                        }
                        OnboardingPrimaryButton(text = "Done", onClick = onDone)
                    }
                    is OnboardingRestoreState.Failed -> {
                        OnboardingInlineMessage(text = restoreState.message)
                        Spacer(modifier = Modifier.size(18.dp))
                        OnboardingPrimaryButton(text = "Retry", onClick = onRetry)
                        Spacer(modifier = Modifier.size(12.dp))
                        OnboardingSecondaryButton(
                            text = "Continue without backup",
                            onClick = onContinueWithoutBackup,
                        )
                    }
                }

                Spacer(modifier = Modifier.size(28.dp))
            }
        }
    }
}

private fun pluralize(
    count: Int,
    singular: String,
    plural: String,
): String = "$count ${if (count == 1) singular else plural}"

@Composable
internal fun OnboardingExchangeFundingView(
    app: AppManager,
    manager: OnboardingManager,
    onContinue: () -> Unit,
) {
    val walletId = manager.currentWalletId()
    val clipboard = LocalContext.current.getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager
    var addressRaw by remember { mutableStateOf<String?>(null) }
    var addressText by remember { mutableStateOf<String?>(null) }
    var errorMessage by remember { mutableStateOf<String?>(null) }
    var didCopyAddress by remember { mutableStateOf(false) }
    val scrollState = rememberScrollState()

    LaunchedEffect(walletId) {
        addressRaw = null
        addressText = null
        errorMessage = null
        didCopyAddress = false

        if (walletId == null) {
            errorMessage = "Unable to load a deposit address for this wallet."
            return@LaunchedEffect
        }

        try {
            val currentWalletManager = app.getWalletManager(walletId)
            currentWalletManager.firstAddress().use { addressInfo ->
                addressRaw = addressInfo.addressUnformatted()
                addressText =
                    addressInfo.address().use { address ->
                        address.spacedOut()
                    }
            }
            errorMessage = null
        } catch (error: Exception) {
            Log.e("OnboardingExchangeFunding", "failed to load first address", error)
            errorMessage = error.message ?: "Unable to load a deposit address for this wallet."
        }
    }

    OnboardingBackground {
        Column(modifier = Modifier.fillMaxSize()) {
            BoxWithConstraints(
                modifier =
                    Modifier
                        .weight(1f)
                        .fillMaxWidth()
                        .statusBarsPadding(),
            ) {
                Column(
                    modifier =
                        Modifier
                            .fillMaxWidth()
                            .heightIn(min = maxHeight)
                            .verticalScroll(scrollState)
                            .padding(horizontal = 24.dp)
                            .padding(top = 32.dp, bottom = 14.dp),
                ) {
                    Text(
                        text = "Your wallet is ready to fund",
                        color = Color.White,
                        fontSize = 34.sp,
                        lineHeight = 38.sp,
                        fontWeight = FontWeight.SemiBold,
                    )

                    Spacer(modifier = Modifier.size(12.dp))

                    Text(
                        text = "Move your Bitcoin off the exchange and into the wallet you now control.",
                        color = OnboardingTextSecondary,
                        style = MaterialTheme.typography.bodySmall.copy(lineHeight = 18.sp),
                    )

                    Spacer(modifier = Modifier.size(24.dp))

                    when {
                        errorMessage != null -> {
                            OnboardingInlineMessage(text = errorMessage!!)
                        }
                        addressRaw != null && addressText != null -> {
                            val qrBitmap =
                                remember(addressRaw) {
                                    QrCodeGenerator.generate(
                                        text = addressRaw!!,
                                        size = 512,
                                        errorCorrectionLevel = ErrorCorrectionLevel.L,
                                    )
                                }

                            Column(verticalArrangement = Arrangement.spacedBy(18.dp)) {
                                Box(
                                    modifier =
                                        Modifier
                                            .align(Alignment.CenterHorizontally)
                                            .widthIn(max = 320.dp)
                                            .fillMaxWidth()
                                            .clip(RoundedCornerShape(18.dp))
                                            .background(Color.White)
                                            .padding(12.dp),
                                ) {
                                    Image(
                                        bitmap = qrBitmap.asImageBitmap(),
                                        contentDescription = "Deposit address QR",
                                        modifier =
                                            Modifier
                                                .fillMaxWidth()
                                                .aspectRatio(1f),
                                        contentScale = ContentScale.Fit,
                                    )
                                }

                                Column(
                                    modifier =
                                        Modifier
                                            .fillMaxWidth()
                                            .background(OnboardingCardFill, RoundedCornerShape(16.dp))
                                            .border(1.dp, OnboardingCardBorder, RoundedCornerShape(16.dp))
                                            .padding(18.dp),
                                    verticalArrangement = Arrangement.spacedBy(8.dp),
                                ) {
                                    Text(
                                        text = "Deposit address",
                                        color = CoveColor.coveLightGray.copy(alpha = 0.72f),
                                        style = MaterialTheme.typography.labelSmall,
                                        fontWeight = FontWeight.SemiBold,
                                    )
                                    Text(
                                        text = addressText!!,
                                        color = Color.White,
                                        style = MaterialTheme.typography.bodyMedium.copy(lineHeight = 20.sp),
                                    )
                                }

                                OnboardingSecondaryButton(
                                    text = if (didCopyAddress) "Copied" else "Copy Address",
                                    onClick = {
                                        clipboard.setPrimaryClip(ClipData.newPlainText("Bitcoin Address", addressRaw!!))
                                        didCopyAddress = true
                                    },
                                )
                            }
                        }
                        else -> {
                            Column(
                                modifier = Modifier.fillMaxWidth().padding(vertical = 48.dp),
                                horizontalAlignment = Alignment.CenterHorizontally,
                                verticalArrangement = Arrangement.spacedBy(12.dp),
                            ) {
                                CircularProgressIndicator(color = Color.White)
                                Text(
                                    text = "Loading deposit address",
                                    color = Color.White,
                                    style = MaterialTheme.typography.bodyMedium,
                                )
                            }
                        }
                    }
                }
            }

            OnboardingPrimaryButton(
                text = "Continue",
                onClick = onContinue,
                modifier =
                    Modifier
                        .padding(horizontal = 24.dp)
                        .padding(top = 14.dp, bottom = 24.dp)
                        .navigationBarsPadding(),
            )
        }
    }
}
