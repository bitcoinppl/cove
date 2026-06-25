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
import androidx.compose.material.icons.filled.CalendarToday
import androidx.compose.material.icons.filled.Cloud
import androidx.compose.material.icons.filled.Key
import androidx.compose.material.icons.filled.Lock
import androidx.compose.material.icons.filled.Person
import androidx.compose.material.icons.filled.Warning
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
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.tooling.preview.Preview
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import org.bitcoinppl.cove.ui.theme.CoveColor
import org.bitcoinppl.cove.ui.theme.caption
import org.bitcoinppl.cove_core.CloudRestoreProviderHint
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
                text = "This can take a few minutes, please be patient",
                color = OnboardingTextSecondary,
                style = MaterialTheme.typography.bodyLarge,
                textAlign = TextAlign.Center,
            )
        }
    }
}

@Composable
internal fun OnboardingRestoreOfferView(
    warningMessage: String?,
    errorMessage: String?,
    providerHint: CloudRestoreProviderHint?,
    onBack: () -> Unit,
    onRestore: () -> Unit,
    onSkip: () -> Unit,
) {
    val title = if (warningMessage == null) "Google Drive Backup Found" else "Restore from Google Drive"
    val body =
        if (warningMessage == null) {
            "A previous Google Drive backup was found. Restore your wallet securely using your passkey."
        } else {
            null
        }
    var previousErrorMessage by remember { mutableStateOf(errorMessage) }
    val visibleErrorMessage = errorMessage ?: previousErrorMessage

    LaunchedEffect(errorMessage) {
        if (errorMessage != null) {
            previousErrorMessage = errorMessage
        }
    }

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
                        .padding(horizontal = 28.dp, vertical = 12.dp)
                        .padding(top = 52.dp),
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

                if (body != null) {
                    Spacer(modifier = Modifier.size(16.dp))

                    Text(
                        text = body,
                        color = OnboardingTextSecondary,
                        style = MaterialTheme.typography.bodyMedium.copy(lineHeight = 20.sp),
                        textAlign = TextAlign.Center,
                        modifier = Modifier.fillMaxWidth(),
                    )
                }

                Spacer(modifier = Modifier.size(28.dp))

                OnboardingPasskeyCard(providerHint = providerHint)

                AnimatedVisibility(
                    visible = errorMessage != null,
                    enter =
                        fadeIn(animationSpec = tween(durationMillis = 300)) +
                            slideInVertically(
                                animationSpec = tween(durationMillis = 300),
                                initialOffsetY = { -it },
                            ),
                    exit =
                        fadeOut(animationSpec = tween(durationMillis = 300)) +
                            slideOutVertically(
                                animationSpec = tween(durationMillis = 300),
                                targetOffsetY = { -it },
                            ),
                ) {
                    Column {
                        Spacer(modifier = Modifier.size(14.dp))
                        OnboardingRestoreErrorCard(text = visibleErrorMessage.orEmpty())
                    }
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

            OnboardingTopBackButton(
                enabled = true,
                onClick = onBack,
                modifier =
                    Modifier
                        .align(Alignment.TopStart)
                        .statusBarsPadding()
                        .padding(start = 16.dp, top = 12.dp),
            )
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
                style = MaterialTheme.typography.caption,
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

        Row(horizontalArrangement = Arrangement.spacedBy(16.dp), verticalAlignment = Alignment.CenterVertically) {
            Box(
                modifier = Modifier.width(48.dp),
                contentAlignment = Alignment.Center,
            ) {
                Icon(
                    imageVector = Icons.Default.Lock,
                    contentDescription = null,
                    tint = OnboardingGradientLight,
                    modifier = Modifier.size(20.dp),
                )
            }

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
            modifier = Modifier.size(34.dp),
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
                style = MaterialTheme.typography.caption,
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
        onBack = {},
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
        onBack = {},
        onRestore = {},
        onSkip = {},
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
