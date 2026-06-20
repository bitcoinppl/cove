package org.bitcoinppl.cove.flows.OnboardingFlow

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Check
import androidx.compose.material.icons.filled.CloudDownload
import androidx.compose.material.icons.filled.Warning
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.res.pluralStringResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import org.bitcoinppl.cove.R
import org.bitcoinppl.cove_core.CloudBackupRestoreFlow
import org.bitcoinppl.cove_core.OnboardingRestoreState

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
                            text = stringResource(R.string.onboarding_restoring_from_drive),
                            color = Color.White,
                            style = MaterialTheme.typography.headlineSmall,
                            fontWeight = FontWeight.SemiBold,
                            textAlign = TextAlign.Center,
                        )
                        Spacer(modifier = Modifier.size(10.dp))
                        Text(
                            text = stringResource(R.string.onboarding_restore_might_take_minutes),
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
                            text =
                                if (failedCount == 0) {
                                    stringResource(R.string.onboarding_restore_all_set)
                                } else {
                                    stringResource(R.string.onboarding_restore_some_wallets_restored)
                                },
                            color = Color.White,
                            style = MaterialTheme.typography.headlineSmall,
                            fontWeight = FontWeight.SemiBold,
                            textAlign = TextAlign.Center,
                        )
                        Spacer(modifier = Modifier.size(10.dp))
                        Text(
                            text =
                                if (failedCount == 0) {
                                    stringResource(R.string.onboarding_restore_wallets_restored)
                                } else {
                                    pluralStringResource(
                                        R.plurals.onboarding_restore_wallets_failed,
                                        failedCount,
                                        failedCount,
                                    )
                                },
                            color = OnboardingTextSecondary,
                            style = MaterialTheme.typography.bodyMedium,
                            textAlign = TextAlign.Center,
                        )
                    }
                    is OnboardingRestoreState.Failed -> {
                        Text(
                            text = stringResource(R.string.onboarding_restore_failed_title),
                            color = Color.White,
                            fontSize = 34.sp,
                            lineHeight = 38.sp,
                            fontWeight = FontWeight.Bold,
                            textAlign = TextAlign.Center,
                        )
                        Spacer(modifier = Modifier.size(12.dp))
                        Text(
                            text = stringResource(R.string.onboarding_restore_failed_body),
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
                            val failedWallets = report.walletsFailed.toInt()
                            OnboardingInlineMessage(
                                text =
                                    pluralStringResource(
                                        R.plurals.onboarding_restore_wallets_failed_inline,
                                        failedWallets,
                                        failedWallets,
                                    ),
                            )
                            Spacer(modifier = Modifier.size(16.dp))
                        }
                        if (report.labelsFailedWalletNames.isNotEmpty()) {
                            val failedLabels = report.labelsFailedWalletNames.size
                            OnboardingInlineMessage(
                                text =
                                    pluralStringResource(
                                        R.plurals.onboarding_restore_labels_failed_inline,
                                        failedLabels,
                                        failedLabels,
                                    ),
                            )
                            Spacer(modifier = Modifier.size(16.dp))
                        }
                        OnboardingPrimaryButton(text = stringResource(R.string.wallet_send_done), onClick = onDone)
                    }
                    is OnboardingRestoreState.Failed -> {
                        OnboardingInlineMessage(text = stringResource(R.string.onboarding_restore_failed_body))
                        Spacer(modifier = Modifier.size(18.dp))
                        OnboardingPrimaryButton(text = stringResource(R.string.scoped_common_retry), onClick = onRetry)
                        Spacer(modifier = Modifier.size(12.dp))
                        OnboardingSecondaryButton(
                            text = stringResource(R.string.onboarding_continue_without_backup_lower),
                            onClick = onContinueWithoutBackup,
                        )
                    }
                }

                Spacer(modifier = Modifier.size(28.dp))
            }
        }
    }
}
