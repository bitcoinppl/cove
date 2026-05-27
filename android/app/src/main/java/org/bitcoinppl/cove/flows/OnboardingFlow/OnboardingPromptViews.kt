package org.bitcoinppl.cove.flows.OnboardingFlow

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.interaction.MutableInteractionSource
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
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.selection.toggleable
import androidx.compose.foundation.text.ClickableText
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
import androidx.compose.material.icons.filled.Search
import androidx.compose.material.icons.filled.Security
import androidx.compose.material.icons.filled.Storage
import androidx.compose.material.icons.filled.Warning
import androidx.compose.material.icons.filled.WifiOff
import androidx.compose.material.icons.outlined.Cloud
import androidx.compose.material.icons.outlined.Search
import androidx.compose.material3.Checkbox
import androidx.compose.material3.CheckboxDefaults
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.mutableStateListOf
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalUriHandler
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.text.SpanStyle
import androidx.compose.ui.text.buildAnnotatedString
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextDecoration
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.withStyle
import androidx.compose.ui.tooling.preview.Preview
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import org.bitcoinppl.cove.R
import org.bitcoinppl.cove.ui.theme.CoveColor
import org.bitcoinppl.cove_core.CloudRestoreProviderHint
import org.bitcoinppl.cove_core.OnboardingStorageSelection
import java.time.Instant
import java.time.ZoneId
import java.time.format.DateTimeFormatter

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
                text = "This only takes a moment",
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
                    .padding(horizontal = 26.dp)
                    .padding(top = 22.dp, bottom = 24.dp),
        ) {
            Column(
                modifier =
                    Modifier
                        .weight(1f)
                        .verticalScroll(rememberScrollState()),
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
            }

            OnboardingPrimaryButton(
                text = "Agree and Continue",
                onClick = onAgree,
                modifier = Modifier.testTag("onboarding.terms.agree"),
                enabled = allChecked,
            )
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
    providerHint: CloudRestoreProviderHint?,
    onRestore: () -> Unit,
    onSkip: () -> Unit,
) {
    val title = if (warningMessage == null) "Google Drive Backup Found" else "Restore from Google Drive"
    val body =
        if (warningMessage == null) {
            "A previous Google Drive backup was found. Restore your wallet securely using your passkey."
        } else {
            "We couldn't confirm whether a Google Drive backup is available. If you're reinstalling this device, you can still try restoring with your passkey."
    }

    OnboardingBackground {
        BoxWithConstraints(
            modifier =
                Modifier
                    .fillMaxSize()
        ) {
            Column(
                modifier =
                    Modifier
                        .fillMaxWidth()
                        .heightIn(min = maxHeight)
                        .verticalScroll(rememberScrollState())
                        .padding(horizontal = 28.dp, vertical = 12.dp),
                horizontalAlignment = Alignment.CenterHorizontally,
                verticalArrangement = Arrangement.Center,
            ) {
                OnboardingStepIndicator(selected = 1, modifier = Modifier.padding(top = 48.dp))

                Spacer(modifier = Modifier.size(42.dp))

                OnboardingCloudSearchHero()

                Spacer(modifier = Modifier.size(38.dp))

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

                Spacer(modifier = Modifier.size(28.dp))

                OnboardingPasskeyCard(providerHint = providerHint)

                if (warningMessage != null) {
                    Spacer(modifier = Modifier.size(14.dp))
                    OnboardingRestoreWarningCard(text = warningMessage)
                }

                if (errorMessage != null) {
                    Spacer(modifier = Modifier.size(14.dp))
                    OnboardingRestoreErrorCard(text = errorMessage)
                }

                Spacer(modifier = Modifier.size(26.dp))

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
}

@Composable
private fun OnboardingPasskeyCard(providerHint: CloudRestoreProviderHint?) {
    Column(
        modifier =
            Modifier
                .fillMaxWidth()
                .background(OnboardingCardFill, RoundedCornerShape(22.dp))
                .border(1.dp, CoveColor.coveLightGray.copy(alpha = 0.18f), RoundedCornerShape(22.dp))
                .padding(horizontal = 20.dp, vertical = 20.dp),
        verticalArrangement = Arrangement.spacedBy(16.dp),
    ) {
        Box(
            modifier =
                Modifier
                    .background(OnboardingGradientLight.copy(alpha = 0.12f), RoundedCornerShape(999.dp))
                    .padding(horizontal = 12.dp, vertical = 6.dp),
        ) {
            Text(
                text = "Recommended",
                color = OnboardingGradientLight.copy(alpha = 0.92f),
                style = MaterialTheme.typography.labelSmall,
                fontWeight = FontWeight.SemiBold,
            )
        }

        Row(horizontalArrangement = Arrangement.spacedBy(16.dp), verticalAlignment = Alignment.CenterVertically) {
            Box(
                modifier =
                    Modifier
                        .size(48.dp)
                        .background(OnboardingGradientLight.copy(alpha = 0.12f), RoundedCornerShape(13.dp)),
                contentAlignment = Alignment.Center,
            ) {
                Icon(
                    imageVector = Icons.Default.Person,
                    contentDescription = null,
                    tint = OnboardingGradientLight,
                    modifier = Modifier.size(24.dp),
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
                    text = providerHint?.passkeyDisplayName() ?: "Secured with your passkey provider",
                    color = CoveColor.coveLightGray.copy(alpha = 0.58f),
                    style = MaterialTheme.typography.bodySmall,
                )
            }
        }

        if (providerHint != null) {
            OnboardingPasskeyDivider()

            Column(verticalArrangement = Arrangement.spacedBy(14.dp)) {
                Text(
                    text = "Provider Details",
                    color = CoveColor.coveLightGray.copy(alpha = 0.72f),
                    style = MaterialTheme.typography.bodySmall,
                    fontWeight = FontWeight.SemiBold,
                )

                val providerName = providerHint.providerName
                if (providerName != null) {
                    Row(horizontalArrangement = Arrangement.spacedBy(14.dp), verticalAlignment = Alignment.CenterVertically) {
                        ProviderDetailItem(
                            icon = Icons.Default.Key,
                            label = "STORED IN",
                            value = providerName,
                            modifier = Modifier.weight(1f),
                        )

                        Box(
                            modifier =
                                Modifier
                                    .width(1.dp)
                                    .height(46.dp)
                                    .background(CoveColor.coveLightGray.copy(alpha = 0.14f)),
                        )

                        ProviderDetailItem(
                            icon = Icons.Default.CalendarToday,
                            label = "CREATED",
                            value = formatPasskeyProviderDate(providerHint.registeredAt),
                            modifier = Modifier.weight(1f),
                        )
                    }
                } else {
                    ProviderDetailItem(
                        icon = Icons.Default.CalendarToday,
                        label = "CREATED",
                        value = formatPasskeyProviderDate(providerHint.registeredAt),
                        modifier = Modifier.fillMaxWidth(),
                    )
                }
            }

            OnboardingPasskeyDivider()
        }

        Row(horizontalArrangement = Arrangement.spacedBy(14.dp), verticalAlignment = Alignment.CenterVertically) {
            Icon(
                imageVector = Icons.Default.Lock,
                contentDescription = null,
                tint = OnboardingGradientLight,
                modifier = Modifier.size(20.dp),
            )

            Text(
                text = "Your passkey is stored securely by your passkey provider, and your encrypted backup is stored in Google Drive app data.",
                color = OnboardingTextSecondary,
                style = MaterialTheme.typography.bodySmall.copy(lineHeight = 18.sp),
                modifier = Modifier.weight(1f),
            )
        }
    }
}

private fun CloudRestoreProviderHint.passkeyDisplayName(): String =
    "Cove Cloud Backup ($nameSuffix)"

@Composable
private fun OnboardingCloudSearchHero() {
    Box(
        modifier = Modifier.size(118.dp),
        contentAlignment = Alignment.Center,
    ) {
        Box(
            modifier =
                Modifier
                    .size(118.dp)
                    .border(1.dp, OnboardingGradientLight.copy(alpha = 0.16f), CircleShape),
        )

        Box(
            modifier =
                Modifier
                    .size(86.dp)
                    .border(1.dp, OnboardingGradientLight.copy(alpha = 0.26f), CircleShape),
        )

        Box(
            modifier =
                Modifier
                    .size(58.dp)
                    .border(1.5.dp, OnboardingGradientLight.copy(alpha = 0.88f), CircleShape),
        )

        Icon(
            imageVector = Icons.Outlined.Cloud,
            contentDescription = null,
            tint = OnboardingGradientLight,
            modifier = Modifier.size(54.dp),
        )

        Icon(
            imageVector = Icons.Outlined.Search,
            contentDescription = null,
            tint = OnboardingGradientLight,
            modifier =
                Modifier
                    .size(28.dp)
                    .padding(start = 18.dp, top = 12.dp),
        )
    }
}

@Composable
private fun ProviderDetailItem(
    icon: androidx.compose.ui.graphics.vector.ImageVector,
    label: String,
    value: String,
    modifier: Modifier = Modifier,
) {
    Row(
        modifier = modifier,
        horizontalArrangement = Arrangement.spacedBy(10.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Icon(
            imageVector = icon,
            contentDescription = null,
            tint = OnboardingGradientLight,
            modifier = Modifier.size(20.dp),
        )

        Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
            Text(
                text = label,
                color = CoveColor.coveLightGray.copy(alpha = 0.64f),
                style = MaterialTheme.typography.labelSmall,
                fontWeight = FontWeight.SemiBold,
            )
            Text(
                text = value,
                color = Color.White,
                style = MaterialTheme.typography.bodySmall,
                fontWeight = FontWeight.SemiBold,
            )
        }
    }
}

@Composable
private fun OnboardingPasskeyDivider() {
    Box(
        modifier =
            Modifier
                .fillMaxWidth()
                .height(1.dp)
                .background(CoveColor.coveLightGray.copy(alpha = 0.16f)),
    )
}

private fun formatPasskeyProviderDate(registeredAt: ULong): String =
    DateTimeFormatter
        .ofPattern("MMM d, yyyy")
        .withZone(ZoneId.systemDefault())
        .format(Instant.ofEpochSecond(registeredAt.toLong()))

@Preview(showBackground = true, backgroundColor = 0xFF0D1B2A)
@Composable
private fun OnboardingRestoreOfferWithProviderHintPreview() {
    OnboardingRestoreOfferView(
        warningMessage = null,
        errorMessage = null,
        providerHint =
            CloudRestoreProviderHint(
                providerName = "Google Password Manager",
                registeredAt = 1_777_612_800u,
                nameSuffix = "09IX",
            ),
        onRestore = {},
        onSkip = {},
    )
}

@Preview(showBackground = true, backgroundColor = 0xFF0D1B2A)
@Composable
private fun OnboardingRestoreOfferWithProviderDatePreview() {
    OnboardingRestoreOfferView(
        warningMessage = null,
        errorMessage = null,
        providerHint =
            CloudRestoreProviderHint(
                providerName = null,
                registeredAt = 1_777_612_800u,
                nameSuffix = "09IY",
            ),
        onRestore = {},
        onSkip = {},
    )
}

@Composable
private fun OnboardingRestoreWarningCard(text: String) {
    OnboardingRestoreMessageCard(
        text = text,
        icon = Icons.Default.Warning,
        foreground = OnboardingGradientLight.copy(alpha = 0.95f),
        background = OnboardingGradientLight.copy(alpha = 0.08f),
        border = OnboardingGradientLight.copy(alpha = 0.22f),
    )
}

@Composable
private fun OnboardingRestoreErrorCard(text: String) {
    OnboardingRestoreMessageCard(
        text = text,
        icon = Icons.Default.Warning,
        foreground = OnboardingWarning.copy(alpha = 0.95f),
        background = OnboardingWarning.copy(alpha = 0.10f),
        border = OnboardingWarning.copy(alpha = 0.28f),
    )
}

@Composable
private fun OnboardingRestoreMessageCard(
    text: String,
    icon: androidx.compose.ui.graphics.vector.ImageVector,
    foreground: Color,
    background: Color,
    border: Color,
) {
    Surface(
        modifier = Modifier.fillMaxWidth(),
        shape = RoundedCornerShape(18.dp),
        color = background,
        tonalElevation = 0.dp,
        shadowElevation = 0.dp,
        border = androidx.compose.foundation.BorderStroke(1.dp, border),
    ) {
        Row(
            modifier = Modifier.padding(horizontal = 14.dp, vertical = 14.dp),
            horizontalArrangement = Arrangement.spacedBy(10.dp),
            verticalAlignment = Alignment.Top,
        ) {
            Icon(
                imageVector = icon,
                contentDescription = null,
                tint = foreground,
                modifier = Modifier.padding(top = 2.dp).size(16.dp),
            )
            Text(
                text = text,
                color = foreground,
                style = MaterialTheme.typography.bodySmall.copy(lineHeight = 18.sp),
            )
        }
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
