package org.bitcoinppl.cove.cloudbackup

import androidx.compose.foundation.BorderStroke
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
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.selection.toggleable
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Check
import androidx.compose.material.icons.filled.Lock
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.stateDescription
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import org.bitcoinppl.cove.R
import org.bitcoinppl.cove.flows.OnboardingFlow.OnboardingBackground
import org.bitcoinppl.cove.flows.OnboardingFlow.OnboardingCardBorder
import org.bitcoinppl.cove.flows.OnboardingFlow.OnboardingCardFill
import org.bitcoinppl.cove.flows.OnboardingFlow.OnboardingGradientLight
import org.bitcoinppl.cove.flows.OnboardingFlow.OnboardingInlineMessage
import org.bitcoinppl.cove.flows.OnboardingFlow.OnboardingPrimaryButton
import org.bitcoinppl.cove.flows.OnboardingFlow.OnboardingStatusHero
import org.bitcoinppl.cove.flows.OnboardingFlow.OnboardingTextSecondary
import org.bitcoinppl.cove.ui.theme.CoveColor
import org.bitcoinppl.cove.ui.theme.caption
import org.bitcoinppl.cove_core.CloudBackupEnableFlow

internal enum class CloudBackupEnableOnboardingContext {
    STANDARD,
    HARDWARE_IMPORT,
}

@Composable
internal fun CloudBackupEnableOnboardingView(
    onEnable: () -> Unit,
    onCancel: () -> Unit,
    message: String?,
    isBusy: Boolean,
    context: CloudBackupEnableOnboardingContext,
    primaryButtonTitle: String,
    cancelButtonTitle: String? = null,
    cancelButtonLeading: Boolean = false,
) {
    var checks by remember { mutableStateOf(listOf(false, false, false)) }

    val allChecked = checks.all { it }
    val resolvedCancelButtonTitle = cancelButtonTitle ?: stringResource(R.string.action_cancel)

    OnboardingBackground {
        Column(
            modifier =
                Modifier
                    .fillMaxSize()
                    .verticalScroll(rememberScrollState())
                    .padding(horizontal = 24.dp, vertical = 18.dp),
        ) {
            Row(
                modifier = Modifier.fillMaxWidth(),
                horizontalArrangement = if (cancelButtonLeading) Arrangement.Start else Arrangement.End,
            ) {
                Text(
                    text = resolvedCancelButtonTitle,
                    color = Color.White,
                    style = MaterialTheme.typography.titleMedium,
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
                        text = stringResource(R.string.cloud_backup_enable_title),
                        color = Color.White,
                        fontSize = 38.sp,
                        lineHeight = 42.sp,
                        fontWeight = FontWeight.SemiBold,
                    )
                    Text(
                        text = stringResource(R.string.cloud_backup_enable_body),
                        color = OnboardingTextSecondary,
                        style = MaterialTheme.typography.bodySmall.copy(lineHeight = 18.sp),
                    )
                }

                Surface(
                    shape = RoundedCornerShape(10.dp),
                    color = OnboardingCardFill,
                    tonalElevation = 0.dp,
                    shadowElevation = 0.dp,
                    border = BorderStroke(1.dp, OnboardingCardBorder),
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
                                    text = stringResource(R.string.cloud_backup_enable_how_it_works),
                                    color = Color.White,
                                    style = MaterialTheme.typography.bodyMedium,
                                    fontWeight = FontWeight.SemiBold,
                                )
                                Text(
                                    text = stringResource(R.string.cloud_backup_enable_secured_with),
                                    color = CoveColor.coveLightGray.copy(alpha = 0.75f),
                                    style = MaterialTheme.typography.caption,
                                )
                            }
                        }

                        Text(
                            text =
                                when (context) {
                                    CloudBackupEnableOnboardingContext.STANDARD ->
                                        stringResource(R.string.cloud_backup_enable_standard_message)
                                    CloudBackupEnableOnboardingContext.HARDWARE_IMPORT ->
                                        stringResource(R.string.cloud_backup_enable_hardware_message)
                            },
                            color = CoveColor.coveLightGray.copy(alpha = 0.60f),
                            style = MaterialTheme.typography.caption.copy(lineHeight = 18.sp),
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
                        text = stringResource(R.string.cloud_backup_enable_understand_passkey),
                    )
                    OnboardingToggleCard(
                        checked = checks[1],
                        onCheckedChange = { checks = checks.toMutableList().apply { set(1, it) } },
                        text = stringResource(R.string.cloud_backup_enable_understand_google),
                    )
                    OnboardingToggleCard(
                        checked = checks[2],
                        onCheckedChange = { checks = checks.toMutableList().apply { set(2, it) } },
                        text =
                            when (context) {
                                CloudBackupEnableOnboardingContext.STANDARD ->
                                    stringResource(R.string.cloud_backup_enable_understand_offline_backup)
                                CloudBackupEnableOnboardingContext.HARDWARE_IMPORT ->
                                    stringResource(R.string.cloud_backup_enable_understand_hardware_backup)
                            },
                    )
                }

                OnboardingPrimaryButton(
                    text = primaryButtonTitle,
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
internal fun CloudBackupEnableBusyOverlay(enableFlow: CloudBackupEnableFlow?) {
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
            border = BorderStroke(1.dp, Color.White.copy(alpha = 0.08f)),
        ) {
            Column(
                modifier = Modifier.padding(horizontal = 24.dp, vertical = 20.dp),
                horizontalAlignment = Alignment.CenterHorizontally,
                verticalArrangement = Arrangement.spacedBy(14.dp),
            ) {
                CircularProgressIndicator(color = Color.White)
                val (title, subtitle) = cloudBackupEnableBusyCopy(enableFlow)
                Text(
                    text = title,
                    color = Color.White,
                    style = MaterialTheme.typography.bodyLarge,
                    fontWeight = FontWeight.SemiBold,
                    textAlign = TextAlign.Center,
                )
                Text(
                    text = subtitle,
                    color = OnboardingTextSecondary,
                    style = MaterialTheme.typography.bodyMedium,
                    textAlign = TextAlign.Center,
                )
            }
        }
    }
}

@Composable
private fun cloudBackupEnableBusyCopy(enableFlow: CloudBackupEnableFlow?): Pair<String, String> =
    when (enableFlow) {
        CloudBackupEnableFlow.CreatingPasskey ->
            stringResource(R.string.cloud_backup_enable_busy_creating_passkey_title) to
                stringResource(R.string.cloud_backup_enable_busy_continue_message)
        CloudBackupEnableFlow.WaitingForPasskeyAvailability ->
            stringResource(R.string.cloud_backup_enable_busy_checking_passkey_title) to
                stringResource(R.string.cloud_backup_enable_busy_checking_passkey_message)
        is CloudBackupEnableFlow.AwaitingSavedPasskeyConfirmation ->
            stringResource(R.string.cloud_backup_enable_busy_checking_passkey_title) to
                stringResource(R.string.cloud_backup_enable_busy_checking_passkey_message)
        CloudBackupEnableFlow.ConfirmingSavedPasskey ->
            stringResource(R.string.cloud_backup_enable_busy_confirming_passkey_title) to
                stringResource(R.string.cloud_backup_enable_busy_continue_message)
        is CloudBackupEnableFlow.UploadingInitialBackup,
        is CloudBackupEnableFlow.RetryingUploadWithStagedMaterial,
        ->
            stringResource(R.string.cloud_backup_enable_busy_creating_backup_title) to
                stringResource(R.string.cloud_backup_enable_busy_continue_message)
        is CloudBackupEnableFlow.AwaitingForceNewConfirmation,
        is CloudBackupEnableFlow.AwaitingPasskeyChoice,
        CloudBackupEnableFlow.DiscoveringExistingBackup,
        null,
        -> stringResource(R.string.cloud_backup_enable_busy_creating_backup_title) to
            stringResource(R.string.cloud_backup_enable_busy_continue_message)
    }

@Composable
private fun OnboardingToggleCard(
    checked: Boolean,
    onCheckedChange: (Boolean) -> Unit,
    text: String,
) {
    val checkedStateDescription = stringResource(R.string.settings_state_checked)
    val uncheckedStateDescription = stringResource(R.string.settings_state_unchecked)

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
                    stateDescription =
                        if (checked) {
                            checkedStateDescription
                        } else {
                            uncheckedStateDescription
                        }
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
