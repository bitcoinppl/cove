@file:Suppress("PackageNaming")

package org.bitcoinppl.cove.flows.SendFlow.SetAmountScreen

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Close
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.IconButtonDefaults
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.rememberModalBottomSheetState
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.draw.clip
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.testTagsAsResourceId
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import org.bitcoinppl.cove.R
import org.bitcoinppl.cove.utils.toComposeColor
import org.bitcoinppl.cove.views.lockaware.ModalBottomSheet
import org.bitcoinppl.cove_core.TransferDestination
import org.bitcoinppl.cove_core.TransferDestinationStatus
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
        // the sheet is its own window, so the app root's resource id mapping doesn't reach it
        Column(
            modifier =
                Modifier
                    .fillMaxWidth()
                    .semantics { testTagsAsResourceId = true }
                    .verticalScroll(rememberScrollState())
                    .padding(bottom = 24.dp),
        ) {
            TransferDestinationHeader(onClose = onDismiss)
            Spacer(Modifier.height(8.dp))

            destinations.forEachIndexed { index, destination ->
                if (index > 0) {
                    HorizontalDivider(
                        color = MaterialTheme.colorScheme.outlineVariant,
                        modifier = Modifier.padding(start = 52.dp, end = 20.dp),
                    )
                }

                TransferDestinationRow(destination = destination, onSelect = onSelect)
            }
        }
    }
}

@Composable
private fun TransferDestinationHeader(onClose: () -> Unit) {
    Row(
        verticalAlignment = Alignment.CenterVertically,
        modifier = Modifier.fillMaxWidth().padding(start = 20.dp, end = 12.dp),
    ) {
        Text(
            text = stringResource(R.string.title_your_wallets),
            style = MaterialTheme.typography.headlineMedium,
            fontWeight = FontWeight.Bold,
            modifier =
                Modifier
                    .weight(1f)
                    .semantics { heading() }
                    .testTag("sendFlow.transferDestinations.title"),
        )
        IconButton(
            onClick = onClose,
            colors =
                IconButtonDefaults.iconButtonColors(
                    containerColor = MaterialTheme.colorScheme.surfaceContainerHigh,
                ),
            modifier = Modifier.size(36.dp).testTag("sendFlow.transferDestinations.close"),
        ) {
            Icon(
                Icons.Filled.Close,
                contentDescription = stringResource(R.string.cd_close_your_wallets),
                tint = MaterialTheme.colorScheme.onSurfaceVariant,
                modifier = Modifier.size(20.dp),
            )
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

    // a wallet that hasn't finished its first scan can't hand out an address yet, so its row says
    // how to fix that instead of naming its keys
    val detail =
        if (isReady) {
            metadata.identOrFingerprint()
        } else {
            stringResource(R.string.label_transfer_destination_not_scanned)
        }

    Row(
        verticalAlignment = Alignment.CenterVertically,
        modifier =
            Modifier
                .fillMaxWidth()
                .testTag("sendFlow.transferDestination.${metadata.id}")
                .clickable(enabled = isReady) { onSelect(metadata.id) }
                .alpha(if (isReady) 1f else NOT_READY_ROW_ALPHA)
                .padding(horizontal = 20.dp, vertical = 14.dp),
    ) {
        Box(
            modifier =
                Modifier
                    .size(14.dp)
                    .clip(CircleShape)
                    .background(metadata.color.toComposeColor()),
        )
        Spacer(Modifier.width(18.dp))
        Column {
            Text(
                text = metadata.name,
                color = MaterialTheme.colorScheme.onSurface,
                fontSize = 15.sp,
                fontWeight = FontWeight.SemiBold,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
            )
            Spacer(Modifier.height(4.dp))
            Text(
                text = detail,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                fontSize = 13.sp,
                fontFamily = if (isReady) FontFamily.Monospace else FontFamily.Default,
            )
        }
    }
}
