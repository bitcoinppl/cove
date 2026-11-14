@file:Suppress("PackageNaming")

package org.bitcoinppl.cove.flows.SendFlow.SetAmountScreen

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.ListItem
import androidx.compose.material3.ListItemDefaults
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.rememberModalBottomSheetState
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.draw.clip
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import org.bitcoinppl.cove.R
import org.bitcoinppl.cove.utils.toComposeColor
import org.bitcoinppl.cove.views.lockaware.ModalBottomSheet
import org.bitcoinppl.cove_core.TransferDestination
import org.bitcoinppl.cove_core.TransferDestinationStatus
import org.bitcoinppl.cove_core.WalletType
import org.bitcoinppl.cove_core.types.WalletId

private const val NOT_READY_ROW_ALPHA = 0.5f

/**
 * lets the user send to another of their wallets instead of typing or scanning an address
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun TransferDestinationSheet(
    destinations: List<TransferDestination>,
    onSelect: (WalletId) -> Unit,
    onDismiss: () -> Unit,
) {
    ModalBottomSheet(
        onDismissRequest = onDismiss,
        sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true),
    ) {
        Column(
            modifier =
                Modifier
                    .fillMaxWidth()
                    .verticalScroll(rememberScrollState())
                    .padding(bottom = 24.dp),
        ) {
            Text(
                text = stringResource(R.string.title_send_to_my_wallet),
                style = MaterialTheme.typography.titleLarge,
                modifier = Modifier.padding(horizontal = 24.dp),
            )
            Spacer(Modifier.height(4.dp))
            Text(
                text = stringResource(R.string.label_send_to_my_wallet_hint),
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                modifier = Modifier.padding(horizontal = 24.dp),
            )
            Spacer(Modifier.height(8.dp))

            destinations.forEach { destination ->
                TransferDestinationRow(destination = destination, onSelect = onSelect)
            }

            // cove derives a hardware wallet's address from its public key, the device never saw it
            if (destinations.any { it.metadata.walletType == WalletType.COLD }) {
                Spacer(Modifier.height(8.dp))
                Text(
                    text = stringResource(R.string.label_send_to_hardware_wallet_note),
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    modifier = Modifier.padding(horizontal = 24.dp),
                )
            }
        }
    }
}

@Composable
private fun TransferDestinationRow(
    destination: TransferDestination,
    onSelect: (WalletId) -> Unit,
) {
    val metadata = destination.metadata
    val isReady = destination.status == TransferDestinationStatus.READY
    val detail =
        when {
            !isReady -> stringResource(R.string.label_transfer_destination_not_scanned)
            metadata.walletType == WalletType.COLD -> stringResource(R.string.label_hardware_wallet)
            metadata.walletType == WalletType.XPUB_ONLY ||
                metadata.walletType == WalletType.WATCH_ONLY -> stringResource(R.string.label_watch_only)
            else -> null
        }

    ListItem(
        headlineContent = { Text(metadata.name) },
        supportingContent = detail?.let { { Text(it) } },
        leadingContent = {
            Box(
                modifier =
                    Modifier
                        .size(10.dp)
                        .clip(CircleShape)
                        .background(metadata.color.toComposeColor()),
            )
        },
        colors = ListItemDefaults.colors(containerColor = MaterialTheme.colorScheme.surfaceContainerLow),
        modifier =
            Modifier
                .fillMaxWidth()
                .testTag("sendFlow.transferDestination.${metadata.id}")
                .clickable(enabled = isReady) { onSelect(metadata.id) }
                .alpha(if (isReady) 1f else NOT_READY_ROW_ALPHA),
    )
}
