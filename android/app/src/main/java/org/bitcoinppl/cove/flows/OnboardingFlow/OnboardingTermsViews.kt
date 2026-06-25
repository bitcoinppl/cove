package org.bitcoinppl.cove.flows.OnboardingFlow

import androidx.compose.foundation.background
import androidx.compose.foundation.border
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
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.selection.toggleable
import androidx.compose.foundation.text.ClickableText
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Check
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
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.text.SpanStyle
import androidx.compose.ui.text.buildAnnotatedString
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextDecoration
import androidx.compose.ui.text.withStyle
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import org.bitcoinppl.cove.ui.theme.CoveColor

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
                    style = MaterialTheme.typography.bodyMedium,
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
        OnboardingTermsCheckIndicator(
            checked = checked,
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
        OnboardingTermsCheckIndicator(
            checked = checked,
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
private fun OnboardingTermsCheckIndicator(checked: Boolean) {
    Box(
        modifier =
            Modifier
                .size(22.dp)
                .background(
                    color = if (checked) OnboardingGradientLight else Color.Transparent,
                    shape = CircleShape,
                )
                .border(
                    width = 2.dp,
                    color = if (checked) OnboardingGradientLight else OnboardingTextSecondary,
                    shape = CircleShape,
                ),
        contentAlignment = Alignment.Center,
    ) {
        if (checked) {
            Icon(
                imageVector = Icons.Filled.Check,
                contentDescription = null,
                tint = Color.White,
                modifier = Modifier.size(14.dp),
            )
        }
    }
}
