package org.bitcoinppl.cove.flows.OnboardingFlow

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.interaction.MutableInteractionSource
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
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.selection.toggleable
import androidx.compose.foundation.text.ClickableText
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.AccountBalance
import androidx.compose.material.icons.filled.AddCircle
import androidx.compose.material.icons.filled.AutoAwesome
import androidx.compose.material.icons.filled.Cloud
import androidx.compose.material.icons.filled.CloudDownload
import androidx.compose.material.icons.filled.CloudOff
import androidx.compose.material.icons.filled.CurrencyBitcoin
import androidx.compose.material.icons.filled.Download
import androidx.compose.material.icons.filled.PhoneIphone
import androidx.compose.material.icons.filled.Security
import androidx.compose.material.icons.filled.Storage
import androidx.compose.material.icons.filled.Warning
import androidx.compose.material.icons.filled.WifiOff
import androidx.compose.material3.Checkbox
import androidx.compose.material3.CheckboxDefaults
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.mutableStateListOf
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalUriHandler
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.text.SpanStyle
import androidx.compose.ui.text.buildAnnotatedString
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextDecoration
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.withStyle
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import org.bitcoinppl.cove.R
import org.bitcoinppl.cove.ui.theme.CoveColor
import org.bitcoinppl.cove_core.OnboardingSoftwareSelection
import org.bitcoinppl.cove_core.OnboardingStorageSelection

@Composable
internal fun CloudCheckContent() {
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
                icon = Icons.Default.Cloud,
                pulse = true,
            )

            Spacer(modifier = Modifier.size(44.dp))

            Text(
                text = "Looking for Google Drive backup...",
                color = Color.White,
                style = MaterialTheme.typography.headlineSmall,
                fontWeight = FontWeight.SemiBold,
                textAlign = TextAlign.Center,
            )

            Spacer(modifier = Modifier.size(10.dp))

            Text(
                text = "Cove may ask for Google Drive access so it can check whether you already have a backup",
                color = OnboardingTextSecondary,
                style = MaterialTheme.typography.bodyMedium,
                textAlign = TextAlign.Center,
            )
        }
    }
}

@Composable
internal fun OnboardingTermsScreen(
    errorMessage: String?,
    onAgree: () -> Unit,
) {
    val uriHandler = LocalUriHandler.current
    val checks = remember { mutableStateListOf(false, false, false, false, false) }
    val allChecked = checks.all { it }

    OnboardingBackground {
        Column(
            modifier =
                Modifier
                    .fillMaxSize()
                    .statusBarsPadding()
                    .navigationBarsPadding()
                    .verticalScroll(rememberScrollState())
                    .padding(horizontal = 26.dp, vertical = 22.dp),
        ) {
            Text(
                text = "Terms & Conditions",
                color = Color.White,
                fontSize = 34.sp,
                lineHeight = 38.sp,
                fontWeight = FontWeight.Bold,
            )

            Spacer(modifier = Modifier.size(12.dp))

            Text(
                text = "By continuing, you agree to the following:",
                color = OnboardingTextSecondary,
                style = MaterialTheme.typography.bodyMedium,
            )

            Spacer(modifier = Modifier.size(20.dp))

            Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                OnboardingTermsCheckboxCard(
                    checked = checks[0],
                    onCheckedChange = { checks[0] = it },
                    text = "I understand that I am responsible for securely managing and backing up my wallets. Cove does not store or recover wallet information.",
                    modifier = Modifier.testTag("onboarding.terms.check.backup"),
                )
                OnboardingTermsCheckboxCard(
                    checked = checks[1],
                    onCheckedChange = { checks[1] = it },
                    text = "I understand that any unlawful use of Cove is strictly prohibited.",
                    modifier = Modifier.testTag("onboarding.terms.check.legal"),
                )
                OnboardingTermsCheckboxCard(
                    checked = checks[2],
                    onCheckedChange = { checks[2] = it },
                    text = "I understand that Cove is not a bank, exchange, or licensed financial institution, and does not offer financial services.",
                    modifier = Modifier.testTag("onboarding.terms.check.financial"),
                )
                OnboardingTermsCheckboxCard(
                    checked = checks[3],
                    onCheckedChange = { checks[3] = it },
                    text = "I understand that if I lose access to my wallet, Cove cannot recover my funds or credentials.",
                    modifier = Modifier.testTag("onboarding.terms.check.recovery"),
                )
                OnboardingTermsAgreementCard(
                    checked = checks[4],
                    onCheckedChange = { checks[4] = it },
                    onOpenUrl = { uriHandler.openUri(it) },
                    modifier = Modifier.testTag("onboarding.terms.check.agreement"),
                )
            }

            Spacer(modifier = Modifier.size(16.dp))

            if (errorMessage != null) {
                OnboardingInlineMessage(text = errorMessage)
                Spacer(modifier = Modifier.size(8.dp))
            }

            Text(
                text = "By checking these boxes, you accept and agree to the above terms.",
                color = CoveColor.coveLightGray.copy(alpha = 0.50f),
                style = MaterialTheme.typography.bodySmall,
            )

            Spacer(modifier = Modifier.size(20.dp))

            OnboardingPrimaryButton(
                text = "Agree and Continue",
                onClick = onAgree,
                modifier = Modifier.testTag("onboarding.terms.agree"),
                enabled = allChecked,
            )

            Spacer(modifier = Modifier.size(24.dp))
        }
    }
}

@Composable
private fun OnboardingTermsCheckboxCard(
    checked: Boolean,
    onCheckedChange: (Boolean) -> Unit,
    text: String,
    modifier: Modifier = Modifier,
) {
    Row(
        modifier =
            modifier
                .fillMaxWidth()
                .background(OnboardingCardFill, RoundedCornerShape(16.dp))
                .toggleable(
                    value = checked,
                    role = Role.Checkbox,
                    interactionSource = remember { MutableInteractionSource() },
                    indication = null,
                    onValueChange = onCheckedChange,
                )
                .padding(horizontal = 16.dp, vertical = 16.dp),
        horizontalArrangement = Arrangement.spacedBy(14.dp),
        verticalAlignment = Alignment.Top,
    ) {
        Checkbox(
            checked = checked,
            onCheckedChange = null,
            colors =
                CheckboxDefaults.colors(
                    checkedColor = OnboardingGradientLight,
                    uncheckedColor = OnboardingTextSecondary,
                    checkmarkColor = Color.White,
                ),
            modifier = Modifier.size(22.dp),
        )
        Text(
            text = text,
            color = Color.White.copy(alpha = 0.82f),
            style = MaterialTheme.typography.bodySmall.copy(lineHeight = 18.sp),
        )
    }
}

@Composable
private fun OnboardingTermsAgreementCard(
    checked: Boolean,
    onCheckedChange: (Boolean) -> Unit,
    onOpenUrl: (String) -> Unit,
    modifier: Modifier = Modifier,
) {
    val text =
        remember {
            buildAnnotatedString {
                append("I have read and agree to Cove's ")
                pushStringAnnotation(tag = "URL", annotation = "https://covebitcoinwallet.com/privacy")
                withStyle(
                    SpanStyle(
                        color = OnboardingGradientLight,
                        textDecoration = TextDecoration.Underline,
                        fontWeight = FontWeight.Bold,
                    ),
                ) {
                    append("Privacy Policy")
                }
                pop()
                append(" and ")
                pushStringAnnotation(tag = "URL", annotation = "https://covebitcoinwallet.com/terms")
                withStyle(
                    SpanStyle(
                        color = OnboardingGradientLight,
                        textDecoration = TextDecoration.Underline,
                        fontWeight = FontWeight.Bold,
                    ),
                ) {
                    append("Terms & Conditions")
                }
                pop()
                append(" as a condition of use.")
            }
        }

    Row(
        modifier =
            modifier
                .fillMaxWidth()
                .background(OnboardingCardFill, RoundedCornerShape(16.dp))
                .toggleable(
                    value = checked,
                    role = Role.Checkbox,
                    interactionSource = remember { MutableInteractionSource() },
                    indication = null,
                    onValueChange = onCheckedChange,
                )
                .padding(horizontal = 16.dp, vertical = 16.dp),
        horizontalArrangement = Arrangement.spacedBy(14.dp),
        verticalAlignment = Alignment.Top,
    ) {
        Checkbox(
            checked = checked,
            onCheckedChange = null,
            colors =
                CheckboxDefaults.colors(
                    checkedColor = OnboardingGradientLight,
                    uncheckedColor = OnboardingTextSecondary,
                    checkmarkColor = Color.White,
                ),
            modifier = Modifier.size(22.dp),
        )

        ClickableText(
            text = text,
            style = MaterialTheme.typography.bodySmall.copy(
                color = Color.White.copy(alpha = 0.82f),
                lineHeight = 18.sp,
            ),
            onClick = { offset ->
                val url = text.getStringAnnotations(tag = "URL", start = offset, end = offset)
                    .firstOrNull()
                if (url == null) {
                    onCheckedChange(!checked)
                } else {
                    onOpenUrl(url.item)
                }
            },
        )
    }
}

@Composable
internal fun OnboardingRestoreOfferView(
    warningMessage: String?,
    errorMessage: String?,
    onRestore: () -> Unit,
    onSkip: () -> Unit,
) {
    val title = if (warningMessage == null) "Google Drive Backup Found" else "Restore from Google Drive"
    val body =
        if (warningMessage == null) {
            "A previous Cove backup was found in Google Drive. Restore your wallet securely using your passkey."
        } else {
            "We couldn't confirm whether a Google Drive backup is available. If you're reinstalling this device, you can still try restoring with your passkey."
        }

    OnboardingBackground {
        Column(
            modifier =
                Modifier
                    .fillMaxSize()
                    .padding(horizontal = 28.dp, vertical = 12.dp),
        ) {
            OnboardingStepIndicator(selected = 1)

            Spacer(modifier = Modifier.size(42.dp))

            OnboardingStatusHero(icon = Icons.Default.CloudDownload)

            Spacer(modifier = Modifier.size(44.dp))

            Text(
                text = title,
                color = Color.White,
                fontSize = 34.sp,
                lineHeight = 38.sp,
                fontWeight = FontWeight.Bold,
                textAlign = TextAlign.Center,
                modifier = Modifier.fillMaxWidth(),
            )

            Spacer(modifier = Modifier.size(16.dp))

            Text(
                text = body,
                color = OnboardingTextSecondary,
                style = MaterialTheme.typography.bodyMedium.copy(lineHeight = 20.sp),
                textAlign = TextAlign.Center,
                modifier = Modifier.fillMaxWidth(),
            )

            Spacer(modifier = Modifier.size(32.dp))

            OnboardingPasskeyCard()

            if (warningMessage != null) {
                Spacer(modifier = Modifier.size(14.dp))
                OnboardingInlineMessage(text = warningMessage)
            }

            if (errorMessage != null) {
                Spacer(modifier = Modifier.size(14.dp))
                OnboardingInlineMessage(text = errorMessage)
            }

            Spacer(modifier = Modifier.weight(1f, fill = true))

            OnboardingPrimaryButton(
                text = "Restore with Passkey",
                onClick = onRestore,
            )

            Spacer(modifier = Modifier.size(16.dp))

            Text(
                text = "Set Up as New",
                color = OnboardingGradientLight.copy(alpha = 0.95f),
                style = MaterialTheme.typography.bodyMedium,
                fontWeight = FontWeight.SemiBold,
                textAlign = TextAlign.Center,
                modifier =
                    Modifier
                        .fillMaxWidth()
                        .clickable(onClick = onSkip)
                        .padding(vertical = 8.dp),
            )

            Spacer(modifier = Modifier.size(26.dp))
        }
    }
}

@Composable
private fun OnboardingPasskeyCard() {
    Column(
        modifier =
            Modifier
                .fillMaxWidth()
                .background(OnboardingCardFill, RoundedCornerShape(22.dp))
                .padding(horizontal = 18.dp, vertical = 18.dp),
        verticalArrangement = Arrangement.spacedBy(16.dp),
    ) {
        Box(
            modifier =
                Modifier
                    .background(OnboardingGradientLight.copy(alpha = 0.12f), RoundedCornerShape(999.dp))
                    .padding(horizontal = 10.dp, vertical = 5.dp),
        ) {
            Text(
                text = "Recommended",
                color = OnboardingGradientLight.copy(alpha = 0.92f),
                style = MaterialTheme.typography.labelSmall,
                fontWeight = FontWeight.SemiBold,
            )
        }

        Row(horizontalArrangement = Arrangement.spacedBy(14.dp), verticalAlignment = Alignment.CenterVertically) {
            Box(
                modifier =
                    Modifier
                        .size(42.dp)
                        .background(OnboardingGradientLight.copy(alpha = 0.12f), RoundedCornerShape(12.dp)),
                contentAlignment = Alignment.Center,
            ) {
                Icon(
                    imageVector = Icons.Default.Security,
                    contentDescription = null,
                    tint = OnboardingGradientLight,
                )
            }

            Column(modifier = Modifier.weight(1f)) {
                Text(
                    text = "Passkey Restore",
                    color = Color.White,
                    style = MaterialTheme.typography.bodyLarge,
                    fontWeight = FontWeight.SemiBold,
                )
                Spacer(modifier = Modifier.size(4.dp))
                Text(
                    text = "Secured with your Google account and passkey",
                    color = CoveColor.coveLightGray.copy(alpha = 0.58f),
                    style = MaterialTheme.typography.bodySmall,
                )
            }
        }

        Text(
            text = "Your passkey is stored securely by your passkey provider, and your encrypted backup is stored in Google Drive app data.",
            color = OnboardingTextSecondary,
            style = MaterialTheme.typography.bodySmall.copy(lineHeight = 18.sp),
        )
    }
}

@Composable
internal fun OnboardingWelcomeScreen(
    errorMessage: String?,
    onContinue: () -> Unit,
) {
    OnboardingPromptScreen(
        icon = Icons.Default.AutoAwesome,
        title = "Welcome to Cove",
        subtitle = "A self-custody Bitcoin wallet focused on secure backups, clear flows, and hardware wallet support.",
    ) {
        if (errorMessage != null) {
            OnboardingInlineMessage(text = errorMessage)
            Spacer(modifier = Modifier.size(14.dp))
        }

        OnboardingPrimaryButton(
            text = "Get Started",
            onClick = onContinue,
            modifier = Modifier.testTag("onboarding.getStarted"),
        )
    }
}

@Composable
internal fun OnboardingBitcoinChoiceScreen(
    errorMessage: String?,
    onNewHere: () -> Unit,
    onHasBitcoin: () -> Unit,
) {
    OnboardingPromptScreen(
        icon = Icons.Default.CurrencyBitcoin,
        title = "Do you already have Bitcoin?",
        subtitle = "We'll tailor the setup based on where you're starting from.",
    ) {
        if (errorMessage != null) {
            OnboardingInlineMessage(text = errorMessage)
            Spacer(modifier = Modifier.size(14.dp))
        }

        Column(verticalArrangement = Arrangement.spacedBy(14.dp)) {
            OnboardingChoiceCard(
                title = "No, I'm new here",
                subtitle = "Create a new wallet and learn the basics",
                icon = Icons.Default.AutoAwesome,
                onClick = onNewHere,
                modifier = Modifier.testTag("onboarding.bitcoinChoice.new"),
            )
            OnboardingChoiceCard(
                title = "Yes, I have Bitcoin",
                subtitle = "Import or connect the wallet you already use",
                icon = Icons.Default.Download,
                onClick = onHasBitcoin,
                modifier = Modifier.testTag("onboarding.bitcoinChoice.existing"),
            )
        }
    }
}

@Composable
internal fun OnboardingReturningUserChoiceScreen(
    onRestoreFromCoveBackup: () -> Unit,
    onUseAnotherWallet: () -> Unit,
    onBack: () -> Unit,
) {
    OnboardingPromptScreen(
        icon = Icons.Default.Download,
        title = "How would you like to continue?",
        subtitle = "Restore from an existing Cove backup or connect another wallet you already use.",
    ) {
        Column(verticalArrangement = Arrangement.spacedBy(14.dp)) {
            OnboardingChoiceCard(
                title = stringResource(R.string.onboarding_restore_card_title),
                subtitle = stringResource(R.string.onboarding_restore_card_subtitle),
                icon = Icons.Default.CloudDownload,
                onClick = onRestoreFromCoveBackup,
            )
            OnboardingChoiceCard(
                title = "Use another wallet",
                subtitle = "Import or connect a wallet from somewhere else",
                icon = Icons.Default.Storage,
                onClick = onUseAnotherWallet,
                modifier = Modifier.testTag("onboarding.returningUser.anotherWallet"),
            )
        }

        Spacer(modifier = Modifier.size(14.dp))

        OnboardingSecondaryButton(
            text = "Back",
            onClick = onBack,
        )
    }
}

@Composable
internal fun OnboardingRestoreUnavailableScreen(
    onContinue: () -> Unit,
    onBack: () -> Unit,
) {
    OnboardingPromptScreen(
        icon = Icons.Default.CloudOff,
        title = "No Google Drive Backup Found",
        subtitle = "We couldn't find a Cove backup in Google Drive for this account. You can continue without cloud restore or go back.",
    ) {
        OnboardingPrimaryButton(
            text = "Continue Without Cloud Restore",
            onClick = onContinue,
        )
        Spacer(modifier = Modifier.size(14.dp))
        OnboardingSecondaryButton(
            text = "Back",
            onClick = onBack,
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
        title = "You're Offline",
        subtitle = "Cove can't check for a Google Drive backup right now. You can continue onboarding and check Cloud Backup later in Settings.",
    ) {
        OnboardingPrimaryButton(
            text = "Continue Without Cloud Restore",
            onClick = onContinue,
        )
        Spacer(modifier = Modifier.size(14.dp))
        OnboardingSecondaryButton(
            text = "Back",
            onClick = onBack,
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
        title = "How do you store your Bitcoin?",
        subtitle = "Choose the option that best matches what you use today.",
    ) {
        if (errorMessage != null) {
            OnboardingInlineMessage(text = errorMessage)
            Spacer(modifier = Modifier.size(14.dp))
        }

        Column(verticalArrangement = Arrangement.spacedBy(14.dp)) {
            if (onRestoreFromCoveBackup != null) {
                OnboardingCloudRestoreChoiceCard(onClick = onRestoreFromCoveBackup)
            }
            OnboardingChoiceCard(
                title = "On an exchange",
                subtitle = "Move funds into a wallet you control",
                icon = Icons.Default.AccountBalance,
                onClick = { onSelectStorage(OnboardingStorageSelection.EXCHANGE) },
                modifier = Modifier.testTag("onboarding.storage.exchange"),
            )
            OnboardingChoiceCard(
                title = "Hardware wallet",
                subtitle = "Import a watch-only wallet from an existing device",
                icon = Icons.Default.Security,
                onClick = { onSelectStorage(OnboardingStorageSelection.HARDWARE_WALLET) },
                modifier = Modifier.testTag("onboarding.storage.hardware"),
            )
            OnboardingChoiceCard(
                title = "Software wallet",
                subtitle = "Import recovery data from another wallet app",
                icon = Icons.Default.PhoneIphone,
                onClick = { onSelectStorage(OnboardingStorageSelection.SOFTWARE_WALLET) },
                modifier = Modifier.testTag("onboarding.storage.software"),
            )
        }

        Spacer(modifier = Modifier.size(14.dp))

        OnboardingSecondaryButton(
            text = "Back",
            onClick = onBack,
            modifier = Modifier.testTag("onboarding.back"),
        )
    }
}

@Composable
internal fun OnboardingSoftwareChoiceScreen(
    errorMessage: String?,
    onRestoreFromCoveBackup: (() -> Unit)?,
    onSelectSoftwareAction: (OnboardingSoftwareSelection) -> Unit,
    onBack: () -> Unit,
) {
    OnboardingPromptScreen(
        icon = Icons.Default.PhoneIphone,
        title = "What would you like to do?",
        subtitle = "Create a new wallet in Cove or import the one you already use.",
    ) {
        if (errorMessage != null) {
            OnboardingInlineMessage(text = errorMessage)
            Spacer(modifier = Modifier.size(14.dp))
        }

        Column(verticalArrangement = Arrangement.spacedBy(14.dp)) {
            if (onRestoreFromCoveBackup != null) {
                OnboardingCloudRestoreChoiceCard(onClick = onRestoreFromCoveBackup)
            }
            OnboardingChoiceCard(
                title = "Create a new wallet",
                subtitle = "Generate a fresh 12-word recovery phrase",
                icon = Icons.Default.AddCircle,
                onClick = { onSelectSoftwareAction(OnboardingSoftwareSelection.CREATE_NEW_WALLET) },
                modifier = Modifier.testTag("onboarding.software.create"),
            )
            OnboardingChoiceCard(
                title = "Import existing wallet",
                subtitle = "Use words or QR from another wallet",
                icon = Icons.Default.Download,
                onClick = { onSelectSoftwareAction(OnboardingSoftwareSelection.IMPORT_EXISTING_WALLET) },
                modifier = Modifier.testTag("onboarding.software.import"),
            )
        }

        Spacer(modifier = Modifier.size(14.dp))

        OnboardingSecondaryButton(
            text = "Back",
            onClick = onBack,
        )
    }
}
