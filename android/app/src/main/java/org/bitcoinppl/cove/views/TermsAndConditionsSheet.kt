package org.bitcoinppl.cove.views

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.ClickableText
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.CheckCircle
import androidx.compose.material.icons.outlined.Circle
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.mutableStateListOf
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalUriHandler
import androidx.compose.ui.text.SpanStyle
import androidx.compose.ui.text.buildAnnotatedString
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextDecoration
import androidx.compose.ui.text.withStyle
import androidx.compose.ui.tooling.preview.Preview
import androidx.compose.ui.unit.dp
import org.bitcoinppl.cove.AppManager
import org.bitcoinppl.cove.ui.theme.CoveColor
import org.bitcoinppl.cove.ui.theme.CoveTheme

@Composable
fun TermsCheckboxItem(
    checked: Boolean,
    onCheckedChange: (Boolean) -> Unit,
    content: @Composable () -> Unit,
) {
    val isDark = isSystemInDarkTheme()
    val backgroundColor =
        if (isDark) {
            MaterialTheme.colorScheme.surfaceContainerHigh
        } else {
            MaterialTheme.colorScheme.surfaceContainerHighest
        }

    Row(
        modifier =
            Modifier
                .fillMaxWidth()
                .background(
                    color = backgroundColor,
                    shape = RoundedCornerShape(10.dp),
                ).clickable(
                    interactionSource = remember { MutableInteractionSource() },
                    indication = null,
                ) { onCheckedChange(!checked) }
                .padding(horizontal = 16.dp, vertical = 20.dp),
        horizontalArrangement = Arrangement.spacedBy(18.dp),
        verticalAlignment = Alignment.Top,
    ) {
        Icon(
            imageVector = if (checked) Icons.Filled.CheckCircle else Icons.Outlined.Circle,
            contentDescription = if (checked) "Checked" else "Unchecked",
            tint =
                if (checked) {
                    MaterialTheme.colorScheme.primary
                } else {
                    MaterialTheme.colorScheme.outline
                },
            modifier = Modifier.padding(top = 2.dp),
        )

        content()
    }
}

@Composable
fun TermsAndConditionsSheet(
    app: AppManager,
) {
    val checks = remember { mutableStateListOf(false, false, false, false, false) }
    val allChecked = checks.all { it }

    Surface(
        modifier = Modifier.fillMaxWidth(),
        color = MaterialTheme.colorScheme.background,
    ) {
        Column(
            modifier =
                Modifier
                    .fillMaxWidth()
                    .verticalScroll(rememberScrollState())
                    .padding(24.dp)
                    .navigationBarsPadding(),
        ) {
            // Title
            Text(
                text = "Terms & Conditions",
                style = MaterialTheme.typography.headlineLarge,
                fontWeight = FontWeight.Bold,
                modifier = Modifier.fillMaxWidth(),
            )

            Spacer(modifier = Modifier.height(24.dp))
            HorizontalDivider(color = MaterialTheme.colorScheme.outlineVariant)
            Spacer(modifier = Modifier.height(24.dp))

            // Subtitle
            Text(
                text = "By continuing, you agree to the following",
                style = MaterialTheme.typography.bodyMedium,
                textAlign = TextAlign.Center,
                modifier = Modifier.fillMaxWidth(),
            )

            Spacer(modifier = Modifier.height(24.dp))
            HorizontalDivider(color = MaterialTheme.colorScheme.outlineVariant)
            Spacer(modifier = Modifier.height(24.dp))

            // Checkboxes
            Column(
                verticalArrangement = Arrangement.spacedBy(6.dp),
            ) {
                TermsCheckboxItem(
                    checked = checks[0],
                    onCheckedChange = { checks[0] = it },
                ) {
                    Text(
                        text = "I understand that I am responsible for securely managing and backing up my wallets. Cove does not store or recover wallet information.",
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurface,
                    )
                }

                TermsCheckboxItem(
                    checked = checks[1],
                    onCheckedChange = { checks[1] = it },
                ) {
                    Text(
                        text = "I understand that any unlawful use of Cove is strictly prohibited.",
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurface,
                    )
                }

                TermsCheckboxItem(
                    checked = checks[2],
                    onCheckedChange = { checks[2] = it },
                ) {
                    Text(
                        text = "I understand that Cove is not a bank, exchange, or licensed financial institution, and does not offer financial services.",
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurface,
                    )
                }

                TermsCheckboxItem(
                    checked = checks[3],
                    onCheckedChange = { checks[3] = it },
                ) {
                    Text(
                        text = "I understand that if I lose access to my wallet, Cove cannot recover my funds or credentials.",
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurface,
                    )
                }

                // Fifth checkbox with clickable links
                TermsCheckboxItem(
                    checked = checks[4],
                    onCheckedChange = { checks[4] = it },
                ) {
                    PrivacyAndTermsText()
                }
            }

            Spacer(modifier = Modifier.height(16.dp))

            // Footnote
            Text(
                text = "By checking these boxes, you accept and agree to the above terms.",
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )

            Spacer(modifier = Modifier.height(24.dp))
            HorizontalDivider(color = MaterialTheme.colorScheme.outlineVariant)
            Spacer(modifier = Modifier.height(24.dp))

            // Primary action button
            Button(
                onClick = { app.agreeToTerms() },
                enabled = allChecked,
                modifier = Modifier.fillMaxWidth(),
                colors =
                    ButtonDefaults.buttonColors(
                        containerColor = MaterialTheme.colorScheme.primary,
                        contentColor = MaterialTheme.colorScheme.onPrimary,
                    ),
            ) {
                Text(
                    text = "Agree and Continue",
                    style = MaterialTheme.typography.titleMedium,
                    fontWeight = FontWeight.SemiBold,
                    textAlign = TextAlign.Center,
                    modifier =
                        Modifier
                            .fillMaxWidth()
                            .padding(vertical = 8.dp),
                )
            }
        }
    }
}

@Composable
private fun PrivacyAndTermsText() {
    val uriHandler = LocalUriHandler.current
    val linkColor = CoveColor.LinkBlue

    val annotatedString =
        buildAnnotatedString {
            append("I have read and agree to Cove's ")

            pushStringAnnotation(tag = "URL", annotation = "https://covebitcoinwallet.com/privacy")
            withStyle(
                style =
                    SpanStyle(
                        color = linkColor,
                        fontWeight = FontWeight.Bold,
                        textDecoration = TextDecoration.Underline,
                    ),
            ) {
                append("Privacy Policy")
            }
            pop()

            append(" and ")

            pushStringAnnotation(tag = "URL", annotation = "https://covebitcoinwallet.com/terms")
            withStyle(
                style =
                    SpanStyle(
                        color = linkColor,
                        fontWeight = FontWeight.Bold,
                        textDecoration = TextDecoration.Underline,
                    ),
            ) {
                append("Terms & Conditions")
            }
            pop()

            append(" as a condition of use.")
        }

    ClickableText(
        text = annotatedString,
        style =
            MaterialTheme.typography.bodySmall.copy(
                color = MaterialTheme.colorScheme.onSurface,
            ),
        onClick = { offset ->
            annotatedString
                .getStringAnnotations(tag = "URL", start = offset, end = offset)
                .firstOrNull()
                ?.let { annotation ->
                    uriHandler.openUri(annotation.item)
                }
        },
    )
}

@Preview(showBackground = true)
@Composable
private fun TermsAndConditionsSheetPreview() {
    CoveTheme {
        TermsAndConditionsSheet(app = AppManager.getInstance())
    }
}

@Preview(showBackground = true)
@Composable
private fun TermsCheckboxItemPreview() {
    CoveTheme {
        Column(
            modifier = Modifier.padding(16.dp),
            verticalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            TermsCheckboxItem(checked = false, onCheckedChange = {}) {
                Text("Unchecked item")
            }
            TermsCheckboxItem(checked = true, onCheckedChange = {}) {
                Text("Checked item")
            }
        }
    }
}
