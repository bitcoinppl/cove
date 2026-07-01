package org.bitcoinppl.cove.flows.OnboardingFlow

import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.core.tween
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.slideInVertically
import androidx.compose.animation.slideOutVertically
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.AccountBalance
import androidx.compose.material.icons.filled.AutoAwesome
import androidx.compose.material.icons.filled.CalendarToday
import androidx.compose.material.icons.filled.Cloud
import androidx.compose.material.icons.filled.CloudOff
import androidx.compose.material.icons.filled.CurrencyBitcoin
import androidx.compose.material.icons.filled.Download
import androidx.compose.material.icons.filled.Key
import androidx.compose.material.icons.filled.Lock
import androidx.compose.material.icons.filled.Person
import androidx.compose.material.icons.filled.PhoneIphone
import androidx.compose.material.icons.filled.Security
import androidx.compose.material.icons.filled.Storage
import androidx.compose.material.icons.filled.Warning
import androidx.compose.material.icons.filled.WifiOff
import androidx.compose.material.icons.outlined.Cloud
import androidx.compose.material3.Icon
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
import androidx.compose.ui.tooling.preview.Preview
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import org.bitcoinppl.cove.R
import org.bitcoinppl.cove.ui.theme.CoveColor
import org.bitcoinppl.cove.ui.theme.caption
import org.bitcoinppl.cove_core.CloudRestoreProviderHint
import org.bitcoinppl.cove_core.OnboardingStorageSelection
import java.time.Instant
import java.time.ZoneId
import java.time.format.DateTimeFormatter

@Composable
internal fun OnboardingWelcomeScreen(
    errorMessage: String?,
    onContinue: () -> Unit,
) {
    OnboardingPromptScreen(
        icon = Icons.Default.AutoAwesome,
        title = stringResource(R.string.onboarding_welcome_title),
        subtitle = stringResource(R.string.onboarding_welcome_subtitle),
    ) {
        if (errorMessage != null) {
            OnboardingInlineMessage(text = errorMessage)
            Spacer(modifier = Modifier.size(14.dp))
        }

        OnboardingPrimaryButton(
            text = stringResource(R.string.onboarding_get_started),
            onClick = onContinue,
            modifier = Modifier.testTag("onboarding.getStarted"),
        )
    }
}

@Composable
internal fun OnboardingBitcoinChoiceScreen(
    errorMessage: String?,
    onRestoreFromCoveBackup: () -> Unit,
    onNewHere: () -> Unit,
    onHasBitcoin: () -> Unit,
) {
    OnboardingPromptScreen(
        icon = Icons.Default.CurrencyBitcoin,
        title = stringResource(R.string.onboarding_bitcoin_choice_title),
        subtitle = stringResource(R.string.onboarding_bitcoin_choice_subtitle),
    ) {
        if (errorMessage != null) {
            OnboardingInlineMessage(text = errorMessage)
            Spacer(modifier = Modifier.size(14.dp))
        }

        Column(verticalArrangement = Arrangement.spacedBy(14.dp)) {
            OnboardingChoiceCard(
                title = stringResource(R.string.onboarding_new_here_title),
                subtitle = stringResource(R.string.onboarding_new_here_subtitle),
                icon = Icons.Default.AutoAwesome,
                onClick = onNewHere,
                modifier = Modifier.testTag("onboarding.bitcoinChoice.new"),
            )
            OnboardingChoiceCard(
                title = stringResource(R.string.onboarding_have_bitcoin_title),
                subtitle = stringResource(R.string.onboarding_have_bitcoin_subtitle),
                icon = Icons.Default.Download,
                onClick = onHasBitcoin,
                modifier = Modifier.testTag("onboarding.bitcoinChoice.existing"),
            )

            OnboardingCloudRestoreChoiceSection(
                onClick = onRestoreFromCoveBackup,
                dividerModifier = Modifier.testTag("onboarding.bitcoinChoice.restoreDivider"),
                cardModifier = Modifier.testTag("onboarding.bitcoinChoice.restore"),
            )
        }
    }
}

@Composable
internal fun OnboardingRestoreUnavailableScreen(
    onContinue: () -> Unit,
    onBack: () -> Unit,
) {
    OnboardingPromptScreen(
        icon = Icons.Default.CloudOff,
        title = stringResource(R.string.onboarding_no_backups_title),
        subtitle = stringResource(R.string.onboarding_no_backups_subtitle),
        onBack = onBack,
    ) {
        OnboardingPrimaryButton(
            text = stringResource(R.string.onboarding_continue_without_backup),
            onClick = onContinue,
        )
    }
}

@Composable
internal fun OnboardingRestoreOfflineScreen(
    onContinue: () -> Unit,
    onBack: () -> Unit,
) {
    OnboardingPromptScreen(
        icon = Icons.Default.WifiOff,
        title = stringResource(R.string.onboarding_offline_title),
        subtitle = stringResource(R.string.onboarding_offline_subtitle),
        onBack = onBack,
    ) {
        OnboardingPrimaryButton(
            text = stringResource(R.string.onboarding_continue_without_cloud_restore),
            onClick = onContinue,
        )
    }
}

@Composable
internal fun OnboardingStorageChoiceScreen(
    errorMessage: String?,
    onRestoreFromCoveBackup: (() -> Unit)?,
    onSelectStorage: (OnboardingStorageSelection) -> Unit,
    onBack: () -> Unit,
) {
    OnboardingPromptScreen(
        icon = Icons.Default.Storage,
        title = stringResource(R.string.onboarding_storage_title),
        subtitle = stringResource(R.string.onboarding_storage_subtitle),
        onBack = onBack,
    ) {
        if (errorMessage != null) {
            OnboardingInlineMessage(text = errorMessage)
            Spacer(modifier = Modifier.size(14.dp))
        }

        Column(verticalArrangement = Arrangement.spacedBy(14.dp)) {
            OnboardingChoiceCard(
                title = stringResource(R.string.onboarding_storage_exchange_title),
                subtitle = stringResource(R.string.onboarding_storage_exchange_subtitle),
                icon = Icons.Default.AccountBalance,
                onClick = { onSelectStorage(OnboardingStorageSelection.EXCHANGE) },
                modifier = Modifier.testTag("onboarding.storage.exchange"),
            )
            OnboardingChoiceCard(
                title = stringResource(R.string.onboarding_storage_hardware_title),
                subtitle = stringResource(R.string.onboarding_storage_hardware_subtitle),
                icon = Icons.Default.Security,
                onClick = { onSelectStorage(OnboardingStorageSelection.HARDWARE_WALLET) },
                modifier = Modifier.testTag("onboarding.storage.hardware"),
            )
            OnboardingChoiceCard(
                title = stringResource(R.string.onboarding_storage_software_title),
                subtitle = stringResource(R.string.onboarding_storage_software_subtitle),
                icon = Icons.Default.PhoneIphone,
                onClick = { onSelectStorage(OnboardingStorageSelection.SOFTWARE_WALLET) },
                modifier = Modifier.testTag("onboarding.storage.software"),
            )
            if (onRestoreFromCoveBackup != null) {
                OnboardingCloudRestoreChoiceSection(
                    onClick = onRestoreFromCoveBackup,
                    showDivider = false,
                    title = stringResource(R.string.onboarding_storage_restore_title),
                    subtitle = stringResource(R.string.onboarding_storage_restore_subtitle),
                    cardModifier = Modifier.testTag("onboarding.storage.restore"),
                )
            }
        }
    }
}

@Composable
private fun OnboardingCloudRestoreChoiceSection(
    onClick: () -> Unit,
    dividerModifier: Modifier = Modifier,
    cardModifier: Modifier = Modifier,
    showDivider: Boolean = true,
    title: String? = null,
    subtitle: String? = null,
) {
    Column(verticalArrangement = Arrangement.spacedBy(14.dp)) {
        if (showDivider) {
            OnboardingChoiceDivider(modifier = dividerModifier)
        }

        OnboardingCloudRestoreChoiceCard(
            onClick = onClick,
            modifier = cardModifier,
            title = title,
            subtitle = subtitle,
        )
    }
}

@Composable
private fun OnboardingChoiceDivider(modifier: Modifier = Modifier) {
    Box(
        modifier =
            modifier
                .fillMaxWidth()
                .height(1.dp)
                .background(CoveColor.coveLightGray.copy(alpha = 0.16f)),
    )
}
