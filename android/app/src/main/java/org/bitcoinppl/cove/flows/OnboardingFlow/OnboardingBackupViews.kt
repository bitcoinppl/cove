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
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.lazy.grid.GridCells
import androidx.compose.foundation.lazy.grid.LazyVerticalGrid
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.selection.toggleable
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Check
import androidx.compose.material.icons.filled.CloudDownload
import androidx.compose.material.icons.filled.ContentCopy
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
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.stateDescription
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
import org.bitcoinppl.cove.cloudbackup.CloudBackupManager
import org.bitcoinppl.cove.findActivity
import org.bitcoinppl.cove.ui.theme.CoveColor
import org.bitcoinppl.cove_core.CloudBackupManagerAction
import org.bitcoinppl.cove_core.CloudBackupRestoreProgress
import org.bitcoinppl.cove_core.CloudBackupRestoreReport
import org.bitcoinppl.cove_core.CloudBackupRestoreStage
import org.bitcoinppl.cove_core.CloudBackupStatus
import org.bitcoinppl.cove_core.OnboardingAction
import org.bitcoinppl.cove_core.OnboardingBranch


internal enum class CloudBackupEnableOnboardingContext {
    STANDARD,
    HARDWARE_IMPORT,
}

internal const val RESTORE_TIMEOUT_MESSAGE = "Restore timed out. Please try again."

internal sealed interface OnboardingRestorePhase {
    data object Restoring : OnboardingRestorePhase

    data class Complete(
        val report: CloudBackupRestoreReport,
    ) : OnboardingRestorePhase

    data class Error(
        val message: String,
    ) : OnboardingRestorePhase
}

internal fun combinedRestoreProgress(restoreProgress: CloudBackupRestoreProgress?): Float {
    restoreProgress ?: return 0f

    return when (restoreProgress.stage) {
        CloudBackupRestoreStage.FINDING -> 0f
        CloudBackupRestoreStage.DOWNLOADING -> {
            val total = restoreProgress.total?.toFloat() ?: return 0f
            if (total <= 0f) return 0f
            restoreProgress.completed.toFloat() / (total * 2f)
        }
        CloudBackupRestoreStage.RESTORING -> {
            val total = restoreProgress.total?.toFloat() ?: return 0f
            if (total <= 0f) return 0f
            (total + restoreProgress.completed.toFloat()) / (total * 2f)
        }
    }
}

internal fun resolveRestorePhase(
    status: CloudBackupStatus,
    restoreReport: CloudBackupRestoreReport?,
    currentPhase: OnboardingRestorePhase,
): OnboardingRestorePhase =
    when (status) {
        is CloudBackupStatus.Error -> {
            if (currentPhase is OnboardingRestorePhase.Restoring) {
                OnboardingRestorePhase.Error(status.v1)
            } else {
                currentPhase
            }
        }
        CloudBackupStatus.Enabled -> {
            restoreReport?.let { OnboardingRestorePhase.Complete(it) } ?: currentPhase
        }
        else -> currentPhase
    }

internal fun shouldNotifyRestoreError(
    currentPhase: OnboardingRestorePhase,
    hasDeliveredError: Boolean,
): Boolean = currentPhase is OnboardingRestorePhase.Restoring && !hasDeliveredError

internal fun shouldCompleteOnboardingCloudBackup(
    status: CloudBackupStatus,
    isCloudBackupEnabled: Boolean,
    isConfigured: Boolean,
): Boolean =
    when (status) {
        CloudBackupStatus.Enabled -> true
        else -> isCloudBackupEnabled && isConfigured
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
                icon = Icons.Default.Lock,
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
            Column(
                modifier =
                    Modifier
                        .weight(1f)
                        .statusBarsPadding()
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
                verticalArrangement = Arrangement.spacedBy(12.dp),
            ) {
                OnboardingSecondaryButton(
                    text = "Back",
                    onClick = onBack,
                )

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
    onEnabled: () -> Unit,
    onSkip: () -> Unit,
) {
    when (branch) {
        OnboardingBranch.SOFTWARE_IMPORT -> {
            OnboardingSoftwareImportCloudBackupStepView(
                onEnabled = onEnabled,
                onSkip = onSkip,
            )
        }
        OnboardingBranch.HARDWARE -> {
            OnboardingHardwareImportCloudBackupStepView(
                onEnabled = onEnabled,
                onSkip = onSkip,
            )
        }
        else -> {
            OnboardingCloudBackupDetailsStepView(
                onEnabled = onEnabled,
                onSkip = onSkip,
                context = CloudBackupEnableOnboardingContext.STANDARD,
            )
        }
    }
}

@Composable
private fun OnboardingSoftwareImportCloudBackupStepView(
    onEnabled: () -> Unit,
    onSkip: () -> Unit,
) {
    var showingDetails by remember { mutableStateOf(false) }

    if (showingDetails) {
        OnboardingCloudBackupDetailsStepView(
            onEnabled = onEnabled,
            onSkip = { showingDetails = false },
            context = CloudBackupEnableOnboardingContext.STANDARD,
        )
    } else {
        OnboardingPromptScreen(
            icon = Icons.Default.Lock,
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
    onEnabled: () -> Unit,
    onSkip: () -> Unit,
) {
    var showingDetails by remember { mutableStateOf(false) }

    if (showingDetails) {
        OnboardingCloudBackupDetailsStepView(
            onEnabled = onEnabled,
            onSkip = { showingDetails = false },
            context = CloudBackupEnableOnboardingContext.HARDWARE_IMPORT,
        )
    } else {
        OnboardingPromptScreen(
            icon = Icons.Default.Lock,
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
    onEnabled: () -> Unit,
    onSkip: () -> Unit,
    context: CloudBackupEnableOnboardingContext,
) {
    val backupManager = remember { CloudBackupManager.getInstance() }
    var didComplete by remember { mutableStateOf(false) }
    var isStartingEnable by remember { mutableStateOf(false) }

    val onboardingMessage =
        when (val status = backupManager.status) {
            CloudBackupStatus.UnsupportedPasskeyProvider ->
                "This passkey provider did not confirm support for Cloud Backup. Try another supported provider."
            is CloudBackupStatus.Error -> status.v1
            else -> null
        }
    val isBusy = isStartingEnable || backupManager.status is CloudBackupStatus.Enabling

    fun completeIfEnabled() {
        if (didComplete) return
        if (!shouldCompleteOnboardingCloudBackup(backupManager.status, backupManager.isCloudBackupEnabled, backupManager.isConfigured)) {
            return
        }

        didComplete = true
        onEnabled()
    }

    LaunchedEffect(backupManager.status, backupManager.isCloudBackupEnabled, backupManager.isConfigured) {
        if (backupManager.status !is CloudBackupStatus.Enabling) {
            isStartingEnable = false
        }
        completeIfEnabled()
    }

    Box(modifier = Modifier.fillMaxSize()) {
        CloudBackupEnableOnboardingView(
            onEnable = {
                if (!isBusy) {
                    isStartingEnable = true
                    backupManager.dispatch(CloudBackupManagerAction.EnableCloudBackupNoDiscovery)
                }
            },
            onCancel = onSkip,
            message = onboardingMessage,
            isBusy = isBusy,
            context = context,
        )

        if (isBusy) {
            Box(
                modifier =
                    Modifier
                        .fillMaxSize()
                        .background(Color.Black.copy(alpha = 0.55f)),
                contentAlignment = Alignment.Center,
            ) {
                Surface(
                    shape = RoundedCornerShape(18.dp),
                    color = CoveColor.midnightBlue.copy(alpha = 0.96f),
                    border = androidx.compose.foundation.BorderStroke(1.dp, Color.White.copy(alpha = 0.08f)),
                ) {
                    Column(
                        modifier = Modifier.padding(horizontal = 24.dp, vertical = 20.dp),
                        horizontalAlignment = Alignment.CenterHorizontally,
                        verticalArrangement = Arrangement.spacedBy(14.dp),
                    ) {
                        CircularProgressIndicator(color = Color.White)
                        Text(
                            text = "Waiting for your new passkey to become available...",
                            color = Color.White,
                            style = MaterialTheme.typography.bodyLarge,
                            fontWeight = FontWeight.SemiBold,
                            textAlign = TextAlign.Center,
                        )
                        Text(
                            text = "Cloud Backup will continue automatically",
                            color = OnboardingTextSecondary,
                            style = MaterialTheme.typography.bodyMedium,
                            textAlign = TextAlign.Center,
                        )
                    }
                }
            }
        }
    }
}

@Composable
private fun CloudBackupEnableOnboardingView(
    onEnable: () -> Unit,
    onCancel: () -> Unit,
    message: String?,
    isBusy: Boolean,
    context: CloudBackupEnableOnboardingContext,
) {
    var checks by remember { mutableStateOf(listOf(false, false, false)) }
    val allChecked = checks.all { it }

    OnboardingBackground {
        Column(
            modifier =
                Modifier
                    .fillMaxSize()
                    .verticalScroll(rememberScrollState())
                    .padding(horizontal = 24.dp, vertical = 18.dp),
        ) {
            Row(modifier = Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.End) {
                Text(
                    text = "Cancel",
                    color = Color.White,
                    style = MaterialTheme.typography.bodyMedium,
                    fontWeight = FontWeight.SemiBold,
                    modifier =
                        Modifier
                            .testTag("onboarding.cloudBackup.cancel")
                            .clip(RoundedCornerShape(12.dp))
                            .clickable(enabled = !isBusy, onClick = onCancel)
                            .padding(horizontal = 8.dp, vertical = 4.dp),
                )
            }

            Spacer(modifier = Modifier.size(8.dp))

            Column(verticalArrangement = Arrangement.spacedBy(24.dp), modifier = Modifier.fillMaxWidth()) {
                OnboardingStatusHero(icon = Icons.Default.Lock)

                Column(verticalArrangement = Arrangement.spacedBy(12.dp)) {
                    Text(
                        text = "Cloud Backup",
                        color = Color.White,
                        fontSize = 38.sp,
                        lineHeight = 42.sp,
                        fontWeight = FontWeight.SemiBold,
                    )
                    Text(
                        text = "Cloud Backup is end-to-end encrypted before it leaves your device and stored in Google Drive app data, secured by a passkey that only you control.",
                        color = OnboardingTextSecondary,
                        style = MaterialTheme.typography.bodySmall.copy(lineHeight = 18.sp),
                    )
                }

                Surface(
                    shape = RoundedCornerShape(10.dp),
                    color = OnboardingCardFill,
                    tonalElevation = 0.dp,
                    shadowElevation = 0.dp,
                    border = androidx.compose.foundation.BorderStroke(1.dp, OnboardingCardBorder),
                ) {
                    Column(
                        modifier = Modifier.fillMaxWidth().padding(16.dp),
                        verticalArrangement = Arrangement.spacedBy(12.dp),
                    ) {
                        Row(horizontalArrangement = Arrangement.spacedBy(12.dp), verticalAlignment = Alignment.CenterVertically) {
                            Box(
                                modifier =
                                    Modifier
                                        .size(40.dp)
                                        .background(OnboardingGradientLight.copy(alpha = 0.15f), RoundedCornerShape(8.dp)),
                                contentAlignment = Alignment.Center,
                            ) {
                                Icon(
                                    imageVector = Icons.Default.Lock,
                                    contentDescription = null,
                                    tint = OnboardingGradientLight,
                                )
                            }

                            Column(modifier = Modifier.weight(1f)) {
                                Text(
                                    text = "How It Works",
                                    color = Color.White,
                                    style = MaterialTheme.typography.bodyMedium,
                                    fontWeight = FontWeight.SemiBold,
                                )
                                Text(
                                    text = "Secured with passkey + Google Drive",
                                    color = CoveColor.coveLightGray.copy(alpha = 0.75f),
                                    style = MaterialTheme.typography.bodySmall,
                                )
                            }
                        }

                        Text(
                            text =
                                when (context) {
                                    CloudBackupEnableOnboardingContext.STANDARD ->
                                        "Your wallet backup is end-to-end encrypted before upload and stored in Google Drive app data. Only your passkey can decrypt it, so both are needed to restore your wallets."
                                    CloudBackupEnableOnboardingContext.HARDWARE_IMPORT ->
                                        "This backs up your imported hardware wallet configuration and labels in Google Drive app data, and it also enables backup for compatible wallets you create in Cove later. Your hardware wallet seed and private keys are not backed up by Cove."
                                },
                            color = CoveColor.coveLightGray.copy(alpha = 0.60f),
                            style = MaterialTheme.typography.bodySmall.copy(lineHeight = 18.sp),
                        )
                    }
                }

                if (message != null) {
                    OnboardingInlineMessage(text = message)
                }

                Column(verticalArrangement = Arrangement.spacedBy(6.dp)) {
                    OnboardingToggleCard(
                        checked = checks[0],
                        onCheckedChange = { checks = checks.toMutableList().apply { set(0, it) } },
                        text = "I understand that my passkey is required to access my Cloud Backup. I must not delete my passkey.",
                    )
                    OnboardingToggleCard(
                        checked = checks[1],
                        onCheckedChange = { checks = checks.toMutableList().apply { set(1, it) } },
                        text = "I understand that I need access to my Google account. If I lose access to my passkey or my Google account, my Cloud Backup won't be recoverable.",
                    )
                    OnboardingToggleCard(
                        checked = checks[2],
                        onCheckedChange = { checks = checks.toMutableList().apply { set(2, it) } },
                        text =
                            when (context) {
                                CloudBackupEnableOnboardingContext.STANDARD ->
                                    "I understand that for maximum safety, I should still manually back up my 12 or 24 words offline on pen and paper."
                                CloudBackupEnableOnboardingContext.HARDWARE_IMPORT ->
                                    "I understand that Cloud Backup does not replace the offline backup for my hardware wallet seed or recovery phrase."
                            },
                    )
                }

                OnboardingPrimaryButton(
                    text = "Enable Cloud Backup",
                    onClick = onEnable,
                    modifier = Modifier.testTag("onboarding.cloudBackup.enable"),
                    enabled = allChecked && !isBusy,
                )

                Spacer(modifier = Modifier.size(16.dp))
            }
        }
    }
}

@Composable
private fun OnboardingToggleCard(
    checked: Boolean,
    onCheckedChange: (Boolean) -> Unit,
    text: String,
) {
    Row(
        modifier =
            Modifier
                .fillMaxWidth()
                .background(OnboardingCardFill, RoundedCornerShape(16.dp))
                .clip(RoundedCornerShape(16.dp))
                .toggleable(
                    value = checked,
                    role = Role.Checkbox,
                    onValueChange = onCheckedChange,
                )
                .semantics {
                    stateDescription = if (checked) "checked" else "unchecked"
                }
                .padding(horizontal = 16.dp, vertical = 16.dp),
        horizontalArrangement = Arrangement.spacedBy(18.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Box(
            modifier =
                Modifier
                    .size(24.dp)
                    .clip(RoundedCornerShape(99.dp))
                    .background(
                        if (checked) OnboardingGradientLight else Color.Transparent,
                    )
                    .border(1.dp, if (checked) OnboardingGradientLight else Color.White.copy(alpha = 0.38f), RoundedCornerShape(99.dp)),
            contentAlignment = Alignment.Center,
        ) {
            if (checked) {
                Icon(
                    imageVector = Icons.Default.Check,
                    contentDescription = null,
                    tint = Color.White,
                    modifier = Modifier.size(16.dp),
                )
            }
        }

        Text(
            text = text,
            color = Color.White.copy(alpha = 0.85f),
            style = MaterialTheme.typography.bodySmall.copy(lineHeight = 18.sp),
        )
    }
}

@Composable
internal fun OnboardingRestoreView(
    onComplete: () -> Unit,
    onError: (String) -> Unit,
) {
    val backupManager = remember { CloudBackupManager.getInstance() }
    var phase by remember { mutableStateOf<OnboardingRestorePhase>(OnboardingRestorePhase.Restoring) }
    var hasStartedRestore by remember { mutableStateOf(false) }
    var hasDeliveredCompletion by remember { mutableStateOf(false) }
    var hasDeliveredError by remember { mutableStateOf(false) }
    var timeoutNonce by remember { mutableStateOf(0) }

    fun failRestore(message: String) {
        val shouldNotify = shouldNotifyRestoreError(phase, hasDeliveredError)
        phase = OnboardingRestorePhase.Error(message)
        if (shouldNotify) {
            hasDeliveredError = true
            onError(message)
        }
    }

    fun startRestore() {
        phase = OnboardingRestorePhase.Restoring
        hasStartedRestore = true
        hasDeliveredCompletion = false
        hasDeliveredError = false
        timeoutNonce += 1
        backupManager.dispatch(CloudBackupManagerAction.RestoreFromCloudBackup)
    }

    fun finishRestore() {
        if (hasDeliveredCompletion) return
        hasDeliveredCompletion = true
        onComplete()
    }

    LaunchedEffect(Unit) {
        if (hasStartedRestore) return@LaunchedEffect
        startRestore()
    }

    DisposableEffect(Unit) {
        onDispose {
            if (backupManager.status is CloudBackupStatus.Restoring) {
                backupManager.dispatch(CloudBackupManagerAction.CancelRestore)
            }
        }
    }

    LaunchedEffect(backupManager.status, backupManager.state.restoreReport) {
        val nextPhase = resolveRestorePhase(backupManager.status, backupManager.state.restoreReport, phase)
        if (nextPhase != phase) {
            if (nextPhase is OnboardingRestorePhase.Error) {
                failRestore(nextPhase.message)
            } else {
                phase = nextPhase
            }
        }
    }

    LaunchedEffect(timeoutNonce) {
        if (timeoutNonce == 0) return@LaunchedEffect
        delay(120_000)
        if (phase != OnboardingRestorePhase.Restoring) return@LaunchedEffect
        backupManager.dispatch(CloudBackupManagerAction.CancelRestore)
        failRestore(RESTORE_TIMEOUT_MESSAGE)
    }

    OnboardingRestoreContent(
        phase = phase,
        combinedProgress = combinedRestoreProgress(backupManager.state.restoreProgress),
        onDone = ::finishRestore,
        onRetry = ::startRestore,
    )
}

@Composable
private fun OnboardingRestoreContent(
    phase: OnboardingRestorePhase,
    combinedProgress: Float,
    onDone: () -> Unit,
    onRetry: () -> Unit,
) {
    OnboardingBackground {
        Column(
            modifier =
                Modifier
                    .fillMaxSize()
                    .padding(horizontal = 28.dp, vertical = 18.dp),
            horizontalAlignment = Alignment.CenterHorizontally,
        ) {
            Spacer(modifier = Modifier.weight(1f, fill = true))

            when (phase) {
                OnboardingRestorePhase.Restoring -> OnboardingStatusHero(icon = Icons.Default.CloudDownload)
                is OnboardingRestorePhase.Complete ->
                    OnboardingStatusHero(
                        icon = Icons.Default.Check,
                        tint = OnboardingSuccess,
                        fillColor = OnboardingSuccess.copy(alpha = 0.12f),
                    )
                is OnboardingRestorePhase.Error ->
                    OnboardingStatusHero(
                        icon = Icons.Default.Warning,
                        tint = Color.Red,
                        fillColor = Color.Red.copy(alpha = 0.12f),
                    )
            }

            Spacer(modifier = Modifier.size(44.dp))

            when (phase) {
                OnboardingRestorePhase.Restoring -> {
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
                is OnboardingRestorePhase.Complete -> {
                    Text(
                        text = "You're all set",
                        color = Color.White,
                        style = MaterialTheme.typography.headlineSmall,
                        fontWeight = FontWeight.SemiBold,
                        textAlign = TextAlign.Center,
                    )
                    Spacer(modifier = Modifier.size(10.dp))
                    Text(
                        text = "Your wallets have been restored.",
                        color = OnboardingTextSecondary,
                        style = MaterialTheme.typography.bodyMedium,
                        textAlign = TextAlign.Center,
                    )
                }
                is OnboardingRestorePhase.Error -> {
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

            Spacer(modifier = Modifier.weight(1f, fill = true))

            when (phase) {
                OnboardingRestorePhase.Restoring -> Unit
                is OnboardingRestorePhase.Complete -> {
                    if (phase.report.walletsFailed.toInt() > 0) {
                        OnboardingInlineMessage(text = "${phase.report.walletsFailed} wallet(s) could not be restored")
                        Spacer(modifier = Modifier.size(16.dp))
                    }
                    if (phase.report.labelsFailedWalletNames.isNotEmpty()) {
                        OnboardingInlineMessage(
                            text = "${phase.report.labelsFailedWalletNames.size} restored wallet(s) had labels that could not be imported",
                        )
                        Spacer(modifier = Modifier.size(16.dp))
                    }
                    OnboardingPrimaryButton(text = "Done", onClick = onDone)
                }
                is OnboardingRestorePhase.Error -> {
                    OnboardingInlineMessage(text = phase.message)
                    Spacer(modifier = Modifier.size(18.dp))
                    OnboardingPrimaryButton(text = "Retry", onClick = onRetry)
                }
            }

            Spacer(modifier = Modifier.size(28.dp))
        }
    }
}

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

    LaunchedEffect(walletId) {
        addressRaw = null
        addressText = null
        errorMessage = null

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
        Column(
            modifier =
                Modifier
                    .fillMaxSize()
                    .verticalScroll(rememberScrollState())
                    .padding(horizontal = 24.dp, vertical = 32.dp),
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

                    Surface(
                        shape = RoundedCornerShape(16.dp),
                        color = OnboardingCardFill,
                        tonalElevation = 0.dp,
                        shadowElevation = 0.dp,
                        border = androidx.compose.foundation.BorderStroke(1.dp, OnboardingCardBorder),
                    ) {
                        Column(
                            modifier = Modifier.fillMaxWidth().padding(18.dp),
                            horizontalAlignment = Alignment.CenterHorizontally,
                            verticalArrangement = Arrangement.spacedBy(18.dp),
                        ) {
                            Box(
                                modifier =
                                    Modifier
                                        .fillMaxWidth()
                                        .clip(RoundedCornerShape(16.dp))
                                        .background(Color.White)
                                        .padding(14.dp),
                            ) {
                                Image(
                                    bitmap = qrBitmap.asImageBitmap(),
                                    contentDescription = "Deposit address QR",
                                    modifier = Modifier.fillMaxWidth(),
                                    contentScale = ContentScale.FillWidth,
                                )
                            }

                            Column(modifier = Modifier.fillMaxWidth(), verticalArrangement = Arrangement.spacedBy(8.dp)) {
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
                                text = "Copy Address",
                                onClick = {
                                    clipboard.setPrimaryClip(ClipData.newPlainText("Bitcoin Address", addressRaw!!))
                                },
                            )
                        }
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

            Spacer(modifier = Modifier.size(24.dp))

            OnboardingPrimaryButton(
                text = "Continue",
                onClick = onContinue,
            )
        }
    }
}
