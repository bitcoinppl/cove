package org.bitcoinppl.cove.flows.OnboardingFlow

import androidx.compose.animation.core.RepeatMode
import androidx.compose.animation.core.animateFloat
import androidx.compose.animation.core.infiniteRepeatable
import androidx.compose.animation.core.rememberInfiniteTransition
import androidx.compose.animation.core.tween
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxScope
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.layout.PaddingValues
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
import androidx.compose.foundation.layout.wrapContentHeight
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.KeyboardArrowRight
import androidx.compose.material.icons.filled.Check
import androidx.compose.material.icons.filled.CloudDownload
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.scale
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import org.bitcoinppl.cove.R
import org.bitcoinppl.cove.ui.theme.CoveColor
import org.bitcoinppl.cove.ui.theme.ForceLightStatusBarIcons

internal val OnboardingGradientLight = Color(0xFF5FA3FF)
internal val OnboardingGradientDark = Color(0xFF2C7AC7)
internal val OnboardingTextSecondary = CoveColor.coveLightGray.copy(alpha = 0.74f)
internal val OnboardingCardFill = CoveColor.duskBlue.copy(alpha = 0.48f)
internal val OnboardingCardBorder = CoveColor.coveLightGray.copy(alpha = 0.14f)
internal val OnboardingWarning = Color(0xFFFFB347)
internal val OnboardingSuccess = Color(0xFF7DD195)

@Composable
internal fun OnboardingBackground(
    modifier: Modifier = Modifier,
    content: @Composable BoxScope.() -> Unit,
) {
    ForceLightStatusBarIcons()

    Box(
        modifier =
            modifier
                .fillMaxSize()
                .background(CoveColor.midnightBlue),
    ) {
        Box(
            modifier =
                Modifier
                    .fillMaxSize()
                    .background(
                        Brush.radialGradient(
                            colors = listOf(Color(0xFF2A5A8B), Color(0x801E3A5C), Color.Transparent),
                            center = androidx.compose.ui.geometry.Offset(280f, 140f),
                            radius = 720f,
                        ),
                    ),
        )
        Box(
            modifier =
                Modifier
                    .fillMaxSize()
                    .background(
                        Brush.radialGradient(
                            colors = listOf(Color(0xFF1E4A6B), Color.Transparent),
                            center = androidx.compose.ui.geometry.Offset(920f, 100f),
                            radius = 520f,
                        ),
                    ),
        )

        content()
    }
}

@Composable
internal fun OnboardingPromptScreen(
    icon: ImageVector,
    title: String,
    subtitle: String,
    modifier: Modifier = Modifier,
    onBack: (() -> Unit)? = null,
    backEnabled: Boolean = true,
    topContent: (@Composable ColumnScope.() -> Unit)? = null,
    content: @Composable ColumnScope.() -> Unit,
) {
    OnboardingBackground(modifier = modifier) {
        BoxWithConstraints(
            modifier =
                Modifier
                    .fillMaxSize()
                    .statusBarsPadding()
                    .navigationBarsPadding(),
        ) {
            val backAction = onBack
            val scrollTopPadding = if (backAction == null) 24.dp else 64.dp

            Column(
                modifier =
                    Modifier
                        .fillMaxWidth()
                        .heightIn(min = maxHeight)
                        .verticalScroll(rememberScrollState())
                        .padding(top = scrollTopPadding, bottom = 24.dp),
                verticalArrangement = Arrangement.Center,
            ) {
                topContent?.invoke(this)

                OnboardingStatusHero(
                    icon = icon,
                    modifier = Modifier.align(Alignment.CenterHorizontally),
                    pulse = true,
                )

                Spacer(modifier = Modifier.height(36.dp))

                Column(
                    modifier = Modifier.padding(horizontal = 24.dp),
                    verticalArrangement = Arrangement.spacedBy(12.dp),
                ) {
                    Text(
                        text = title,
                        color = Color.White,
                        fontSize = 34.sp,
                        fontWeight = FontWeight.SemiBold,
                        lineHeight = 38.sp,
                        modifier = Modifier.fillMaxWidth(),
                    )

                    Text(
                        text = subtitle,
                        color = OnboardingTextSecondary,
                        style = MaterialTheme.typography.bodySmall.copy(lineHeight = 18.sp),
                        modifier = Modifier.fillMaxWidth(),
                    )
                }

                Spacer(modifier = Modifier.height(26.dp))

                Column(
                    modifier = Modifier.padding(horizontal = 24.dp),
                ) {
                    content()
                }
            }

            if (backAction != null) {
                OnboardingTopBackButton(
                    enabled = backEnabled,
                    onClick = backAction,
                    modifier =
                        Modifier
                            .align(Alignment.TopStart)
                            .statusBarsPadding()
                            .padding(start = 16.dp, top = 12.dp),
                )
            }
        }
    }
}

@Composable
internal fun OnboardingTopBackButton(
    enabled: Boolean,
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
) {
    TextButton(
        onClick = onClick,
        enabled = enabled,
        modifier = modifier.testTag("onboarding.back"),
        colors =
            ButtonDefaults.textButtonColors(
                contentColor = Color.White,
                disabledContentColor = Color.White.copy(alpha = 0.38f),
            ),
    ) {
        Text(
            text = "Back",
            style = MaterialTheme.typography.titleMedium,
            fontWeight = FontWeight.SemiBold,
        )
    }
}

@Composable
internal fun OnboardingStatusHero(
    icon: ImageVector,
    modifier: Modifier = Modifier,
    tint: Color = OnboardingGradientLight,
    fillColor: Color = CoveColor.duskBlue.copy(alpha = 0.42f),
    pulse: Boolean = false,
) {
    val transition = rememberInfiniteTransition(label = "onboarding_hero")
    val scale = if (pulse) {
        transition.animateFloat(
            initialValue = 0.96f,
            targetValue = 1.06f,
            animationSpec = infiniteRepeatable(tween(1850), RepeatMode.Reverse),
            label = "onboarding_hero_scale",
        ).value
    } else {
        1f
    }

    Box(
        modifier =
            modifier
                .size(118.dp)
                .wrapContentHeight(Alignment.CenterVertically),
        contentAlignment = Alignment.Center,
    ) {
        val ringSizes = remember { listOf(118.dp, 86.dp, 58.dp) }
        val ringAlphas = remember { listOf(0.15f, 0.22f, 0.34f) }

        ringSizes.zip(ringAlphas).forEach { (size, alpha) ->
            Box(
                modifier =
                    Modifier
                        .size(size)
                        .scale(if (pulse) scale else 1f)
                        .border(1.dp, tint.copy(alpha = alpha), CircleShape),
            )
        }

        Box(
            modifier =
                Modifier
                    .size(58.dp)
                    .clip(CircleShape)
                    .background(fillColor)
                    .border(1.3.dp, tint.copy(alpha = 0.72f), CircleShape),
            contentAlignment = Alignment.Center,
        ) {
            Icon(
                imageVector = icon,
                contentDescription = null,
                tint = tint,
                modifier = Modifier.size(24.dp),
            )
        }
    }
}

@Composable
internal fun OnboardingStepIndicator(
    selected: Int,
    total: Int = 3,
    modifier: Modifier = Modifier,
) {
    Row(
        modifier = modifier.fillMaxWidth(),
        horizontalArrangement = Arrangement.Center,
        verticalAlignment = Alignment.CenterVertically,
    ) {
        repeat(total) { index ->
            if (index == selected) {
                Box(
                    modifier =
                        Modifier
                            .width(24.dp)
                            .height(6.dp)
                            .clip(RoundedCornerShape(99.dp))
                            .background(Color.White),
                )
            } else {
                Box(
                    modifier =
                        Modifier
                            .size(6.dp)
                            .clip(CircleShape)
                            .background(Color.White.copy(alpha = 0.22f)),
                )
            }

            if (index < total - 1) {
                Spacer(modifier = Modifier.width(9.dp))
            }
        }
    }
}

@Composable
internal fun OnboardingThinProgressBar(
    progress: Float,
    modifier: Modifier = Modifier,
) {
    Box(
        modifier =
            modifier
                .width(164.dp)
                .height(5.dp)
                .clip(RoundedCornerShape(99.dp))
                .background(Color.White.copy(alpha = 0.12f)),
    ) {
        Box(
            modifier =
                Modifier
                    .fillMaxWidth(progress.coerceIn(0f, 1f))
                    .height(5.dp)
                    .clip(RoundedCornerShape(99.dp))
                    .background(OnboardingGradientLight),
        )
    }
}

@Composable
internal fun OnboardingPrimaryButton(
    text: String,
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
    enabled: Boolean = true,
) {
    Button(
        onClick = onClick,
        enabled = enabled,
        modifier = modifier.fillMaxWidth(),
        shape = RoundedCornerShape(16.dp),
        colors =
            ButtonDefaults.buttonColors(
                containerColor = Color.Transparent,
                contentColor = Color.White,
                disabledContainerColor = Color.Transparent,
                disabledContentColor = Color.White.copy(alpha = 0.45f),
            ),
        contentPadding = PaddingValues(0.dp),
    ) {
        Box(
            modifier =
                Modifier
                    .fillMaxWidth()
                    .clip(RoundedCornerShape(16.dp))
                    .background(
                        Brush.horizontalGradient(
                            colors =
                                if (enabled) {
                                    listOf(OnboardingGradientLight, OnboardingGradientDark)
                                } else {
                                    listOf(
                                        OnboardingGradientLight.copy(alpha = 0.24f),
                                        OnboardingGradientDark.copy(alpha = 0.24f),
                                    )
                                },
                        ),
                    )
                    .padding(vertical = 18.dp, horizontal = 18.dp),
            contentAlignment = Alignment.Center,
        ) {
            Text(
                text = text,
                fontWeight = FontWeight.SemiBold,
                style = MaterialTheme.typography.titleMedium,
                color = Color.White.copy(alpha = if (enabled) 1f else 0.45f),
            )
        }
    }
}

@Composable
internal fun OnboardingSecondaryButton(
    text: String,
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
    enabled: Boolean = true,
) {
    Button(
        onClick = onClick,
        enabled = enabled,
        modifier = modifier.fillMaxWidth(),
        shape = RoundedCornerShape(16.dp),
        colors =
            ButtonDefaults.buttonColors(
                containerColor = CoveColor.duskBlue.copy(alpha = 0.58f),
                contentColor = Color.White,
                disabledContainerColor = CoveColor.duskBlue.copy(alpha = 0.25f),
                disabledContentColor = Color.White.copy(alpha = 0.45f),
            ),
        border = androidx.compose.foundation.BorderStroke(1.dp, OnboardingCardBorder),
        contentPadding = PaddingValues(0.dp),
    ) {
        Text(
            text = text,
            modifier = Modifier.padding(vertical = 17.dp, horizontal = 18.dp),
            style = MaterialTheme.typography.bodyLarge,
            fontWeight = FontWeight.SemiBold,
        )
    }
}

@Composable
internal fun OnboardingChoiceCard(
    title: String,
    subtitle: String,
    icon: ImageVector,
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
) {
    Surface(
        modifier =
            modifier
                .fillMaxWidth()
                .clickable(onClick = onClick),
        shape = RoundedCornerShape(18.dp),
        color = OnboardingCardFill,
        tonalElevation = 0.dp,
        shadowElevation = 0.dp,
        border = androidx.compose.foundation.BorderStroke(1.dp, OnboardingCardBorder),
    ) {
        Row(
            modifier = Modifier.padding(horizontal = 18.dp, vertical = 18.dp),
            horizontalArrangement = Arrangement.spacedBy(14.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Box(
                modifier =
                    Modifier
                        .size(48.dp)
                        .clip(RoundedCornerShape(12.dp))
                        .background(OnboardingGradientLight.copy(alpha = 0.18f)),
                contentAlignment = Alignment.Center,
            ) {
                Icon(
                    imageVector = icon,
                    contentDescription = null,
                    tint = OnboardingGradientLight,
                    modifier = Modifier.size(19.dp),
                )
            }

            Column(modifier = Modifier.weight(1f)) {
                Text(
                    text = title,
                    color = Color.White,
                    fontWeight = FontWeight.SemiBold,
                    style = MaterialTheme.typography.bodyLarge,
                )
                Spacer(modifier = Modifier.height(4.dp))
                Text(
                    text = subtitle,
                    color = OnboardingTextSecondary,
                    style = MaterialTheme.typography.bodySmall.copy(lineHeight = 18.sp),
                )
            }

            Icon(
                imageVector = Icons.AutoMirrored.Filled.KeyboardArrowRight,
                contentDescription = null,
                tint = Color.White.copy(alpha = 0.46f),
                modifier = Modifier.size(18.dp),
            )
        }
    }
}

@Composable
internal fun OnboardingStatusCard(
    title: String,
    subtitle: String,
    actionTitle: String,
    icon: ImageVector,
    isComplete: Boolean,
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
) {
    Surface(
        modifier = modifier.fillMaxWidth(),
        shape = RoundedCornerShape(18.dp),
        color = OnboardingCardFill,
        tonalElevation = 0.dp,
        shadowElevation = 0.dp,
        border = androidx.compose.foundation.BorderStroke(1.dp, OnboardingCardBorder),
    ) {
        Column(modifier = Modifier.padding(18.dp)) {
            Row(
                horizontalArrangement = Arrangement.spacedBy(14.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Box(
                    modifier =
                        Modifier
                            .size(42.dp)
                            .clip(RoundedCornerShape(12.dp))
                            .background(OnboardingGradientLight.copy(alpha = 0.16f)),
                    contentAlignment = Alignment.Center,
                ) {
                    Icon(
                        imageVector = icon,
                        contentDescription = null,
                        tint = OnboardingGradientLight,
                    )
                }

                Column(modifier = Modifier.weight(1f)) {
                    Text(
                        text = title,
                        color = Color.White,
                        fontWeight = FontWeight.SemiBold,
                        style = MaterialTheme.typography.bodyLarge,
                    )
                    Spacer(modifier = Modifier.height(4.dp))
                    Text(
                        text = subtitle,
                        color = OnboardingTextSecondary,
                        style = MaterialTheme.typography.bodySmall.copy(lineHeight = 18.sp),
                    )
                }

                if (isComplete) {
                    Icon(
                        imageVector = Icons.Default.Check,
                        contentDescription = null,
                        tint = OnboardingSuccess,
                        modifier = Modifier.size(20.dp),
                    )
                }
            }

            Spacer(modifier = Modifier.height(14.dp))

            OnboardingSecondaryButton(
                text = actionTitle,
                onClick = onClick,
                enabled = true,
            )
        }
    }
}

@Composable
internal fun OnboardingInlineMessage(
    text: String,
    modifier: Modifier = Modifier,
) {
    Surface(
        modifier = modifier.fillMaxWidth(),
        shape = RoundedCornerShape(14.dp),
        color = Color.Red.copy(alpha = 0.20f),
        tonalElevation = 0.dp,
        shadowElevation = 0.dp,
        border = androidx.compose.foundation.BorderStroke(1.dp, Color.Red.copy(alpha = 0.35f)),
    ) {
        Text(
            text = text,
            color = Color.White,
            style = MaterialTheme.typography.bodySmall.copy(lineHeight = 18.sp),
            modifier = Modifier.padding(14.dp),
        )
    }
}

@Composable
internal fun OnboardingCloudRestoreChoiceCard(
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
    title: String? = null,
    subtitle: String? = null,
) {
    OnboardingChoiceCard(
        title = title ?: stringResource(R.string.onboarding_restore_card_title),
        subtitle = subtitle ?: stringResource(R.string.onboarding_restore_card_subtitle),
        icon = Icons.Default.CloudDownload,
        onClick = onClick,
        modifier = modifier,
    )
}
