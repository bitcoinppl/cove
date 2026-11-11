package org.bitcoinppl.cove.cloudbackup

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Security
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import org.bitcoinppl.cove.ui.theme.caption
import org.bitcoinppl.cove_core.DeepVerificationReport

/** The result of a verification that passed, with what it repaired on the way */
@Composable
internal fun VerifiedSectionContent(
    report: DeepVerificationReport,
) {
    val colors = cloudBackupVisualColors()
    val summaryItems = buildVerifiedSummaryItems(report)

    CloudBackupGlassCard(
        modifier =
            Modifier
                .fillMaxWidth()
                .padding(start = 14.dp, top = 10.dp, end = 14.dp),
        fill = colors.verifiedFill,
        border = colors.verifiedBorder,
    ) {
        Column(
            modifier = Modifier.padding(16.dp),
            verticalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            Row(
                horizontalArrangement = Arrangement.spacedBy(12.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Icon(
                    Icons.Default.Security,
                    contentDescription = null,
                    tint = colors.success,
                    modifier = Modifier.size(32.dp),
                )
                Text(
                    "Backup verified",
                    style = MaterialTheme.typography.titleSmall,
                    fontWeight = FontWeight.SemiBold,
                    color = colors.primaryText,
                )
            }

            VerifiedSummary(
                items = summaryItems,
                color = colors.secondaryText,
            )
        }
    }
}

@Composable
private fun VerifiedSummary(
    items: List<String>,
    color: Color,
) {
    if (items.size > 2) {
        Column(
            modifier = Modifier.fillMaxWidth(),
            verticalArrangement = Arrangement.spacedBy(4.dp),
        ) {
            items.forEach { item ->
                Text(
                    item,
                    style = MaterialTheme.typography.caption,
                    color = color,
                )
            }
        }

        return
    }

    Text(
        items.joinToString(" • "),
        style = MaterialTheme.typography.caption,
        color = color,
    )
}

private fun buildVerifiedSummaryItems(report: DeepVerificationReport): List<String> =
    buildList {
        if (report.credentialRecovered) {
            add("passkey recovered")
        }
        if (report.masterKeyWrapperRepaired) {
            add("cloud master key protection repaired")
        }
        if (report.localMasterKeyRepaired) {
            add("local backup credentials repaired")
        }
        add("${report.walletsVerified} wallet(s) verified")
    }
