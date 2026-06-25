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
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
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
