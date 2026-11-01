@file:Suppress("PackageNaming")

package org.bitcoinppl.cove.flows.SendFlow.Common

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.size
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.CurrencyBitcoin
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import org.bitcoinppl.cove.ui.theme.CoveColor
import org.bitcoinppl.cove.views.BitcoinShieldIcon
import org.bitcoinppl.cove_core.WalletMetadata
import org.bitcoinppl.cove_core.WalletType

internal const val SEND_FLOW_ACCOUNT_HOT_ICON_TAG = "sendFlow.account.hotIcon"
internal const val SEND_FLOW_ACCOUNT_SHIELD_ICON_TAG = "sendFlow.account.shieldIcon"

/**
 * The sending wallet's identity line and name, with an icon for its wallet type
 *
 * Shared by the set amount, confirm and hardware signing screens so every send step names the
 * wallet the funds actually leave from
 */
@Composable
internal fun SendFlowAccountSection(
    metadata: WalletMetadata?,
    modifier: Modifier = Modifier,
) {
    if (metadata == null) return

    Row(
        modifier = modifier,
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        when (metadata.walletType) {
            WalletType.HOT ->
                Icon(
                    Icons.Filled.CurrencyBitcoin,
                    contentDescription = null,
                    tint = CoveColor.WarningOrange,
                    modifier = Modifier.size(28.dp).testTag(SEND_FLOW_ACCOUNT_HOT_ICON_TAG),
                )

            WalletType.COLD, WalletType.XPUB_ONLY ->
                BitcoinShieldIcon(
                    modifier = Modifier.testTag(SEND_FLOW_ACCOUNT_SHIELD_ICON_TAG),
                    size = 24.dp,
                    color = CoveColor.bitcoinOrange,
                )

            // matches iOS, which shows no icon for address-only watch wallets
            WalletType.WATCH_ONLY -> Unit
        }

        Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
            Text(
                metadata.identOrFingerprint(),
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                fontSize = 13.sp,
                fontWeight = FontWeight.Medium,
            )
            Text(
                metadata.name,
                color = MaterialTheme.colorScheme.onSurface,
                fontSize = 13.sp,
                fontWeight = FontWeight.SemiBold,
            )
        }
    }
}
