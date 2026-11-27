@file:Suppress("PackageNaming")

package org.bitcoinppl.cove.flows.FeeBumpFlow

import android.content.ClipData
import android.content.ClipboardManager
import android.content.Context
import android.os.Build
import android.widget.Toast
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Bolt
import androidx.compose.material.icons.filled.NorthEast
import androidx.compose.material.icons.filled.Schedule
import androidx.compose.material.icons.filled.SouthWest
import androidx.compose.material.icons.outlined.ContentCopy
import androidx.compose.material.icons.outlined.Info
import androidx.compose.material.icons.outlined.Tag
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.rememberModalBottomSheetState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.ExperimentalComposeUiApi
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.testTagsAsResourceId
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import org.bitcoinppl.cove.AppManager
import org.bitcoinppl.cove.WalletManager
import org.bitcoinppl.cove.runCatchingCancellable
import org.bitcoinppl.cove.ui.theme.CoveColor
import org.bitcoinppl.cove.views.lockaware.ModalBottomSheet
import org.bitcoinppl.cove_core.PendingSendStatus
import org.bitcoinppl.cove_core.Transaction
import org.bitcoinppl.cove_core.TransactionDetailsPresentation
import org.bitcoinppl.cove_core.types.TransactionDirection
import org.bitcoinppl.cove_core.types.TxId
import java.text.DateFormat
import java.util.Date

private const val TAG = "TransactionActionsSheet"
private const val MILLIS_PER_SECOND = 1_000L
private const val ICON_BACKGROUND_ALPHA = 0.75f
private const val PENDING_ICON_ALPHA = 0.6f
private const val HIDDEN = "••••••"

/**
 * The long-press sheet of a transaction row: a card summing up the transaction, then its actions
 *
 * Speed Up leads, only for a pending send the wallet can bump. The rest are on every row
 */
@OptIn(ExperimentalMaterial3Api::class, ExperimentalComposeUiApi::class)
@Composable
fun TransactionActionsSheet(
    app: AppManager,
    walletManager: WalletManager,
    target: TransactionMenuTarget,
    onSpeedUp: (TxId) -> Unit,
    onDismiss: () -> Unit,
) {
    val summary = remember(target.transaction) { TransactionSummary(target.transaction) }
    var details by remember { mutableStateOf<TransactionDetailsPresentation?>(null) }
    var isLoaded by remember { mutableStateOf(false) }

    CloseOnRouteChange(app, onDismiss)

    LaunchedEffect(summary.txId) {
        details =
            runCatchingCancellable(TAG, "Unable to load details of ${summary.txId}") {
                walletManager.transactionDetails(summary.txId)
            }.getOrNull()

        isLoaded = true
    }

    // opens once the local details lookup ends: a sheet anchored before the address arrives
    // doesn't grow to fit it, which pushed the last action off the screen
    if (!isLoaded) return

    ModalBottomSheet(
        onDismissRequest = onDismiss,
        sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true),
        containerColor = MaterialTheme.colorScheme.surface,
    ) {
        Column(
            modifier =
                Modifier
                    .fillMaxWidth()
                    .semantics { testTagsAsResourceId = true }
                    .testTag("transactionMenu")
                    .padding(bottom = 16.dp),
        ) {
            TransactionSummaryCard(walletManager = walletManager, summary = summary, details = details)

            HorizontalDivider(modifier = Modifier.padding(vertical = 8.dp))

            if (summary.pendingSend?.isBumpable() == true) {
                TransactionActionRow(
                    icon = Icons.Filled.Bolt,
                    title = "Speed Up",
                    subtitle = "Get your transaction confirmed faster",
                    testTag = "transactionMenu.speedUp",
                    onClick = { onSpeedUp(summary.txId) },
                )
            }

            TransactionCopyActions(summary = summary, details = details, onDismiss = onDismiss)

            TransactionActionRow(
                icon = Icons.Outlined.Info,
                title = "View Transaction Details",
                testTag = "transactionMenu.details",
                onClick = {
                    onDismiss()
                    target.openDetails()
                },
            )
        }
    }
}

/** What the card shows of a transaction, read once from its row */
private class TransactionSummary(
    transaction: Transaction,
) {
    val txId: TxId
    val isConfirmed: Boolean
    val label: String
    val pendingSend: PendingSendStatus?
    val direction: TransactionDirection
    val amountSource: org.bitcoinppl.cove_core.types.SentAndReceived
    val fiat: Double?
    val date: String

    init {
        when (transaction) {
            is Transaction.Confirmed -> {
                val txn = transaction.v1
                txId = txn.id()
                isConfirmed = true
                label = txn.label()
                pendingSend = null
                amountSource = txn.sentAndReceived()
                fiat = txn.fiatAmount()?.amount
                date = txn.confirmedAtFmt()
            }

            is Transaction.Unconfirmed -> {
                val txn = transaction.v1
                txId = txn.id()
                isConfirmed = false
                label = txn.label()
                pendingSend = txn.pendingSend()
                amountSource = txn.sentAndReceived()
                fiat = txn.fiatAmount()?.amount
                date =
                    DateFormat
                        .getDateTimeInstance(DateFormat.MEDIUM, DateFormat.SHORT)
                        .format(Date(txn.firstSeen().toLong() * MILLIS_PER_SECOND))
            }
        }

        direction = amountSource.direction()
    }
}

@Composable
private fun TransactionSummaryCard(
    walletManager: WalletManager,
    summary: TransactionSummary,
    details: TransactionDetailsPresentation?,
) {
    val visible = walletManager.walletMetadata?.sensitiveVisible ?: true
    fun privateShow(text: String) = if (visible) text else HIDDEN

    val amount = privateShow(walletManager.displaySentAndReceivedAmount(summary.amountSource))
    val fiat = summary.fiat?.let { privateShow(walletManager.displayFiatAmountWithDirection(it, summary.direction)) }
    val confirmations =
        details?.confirmations()?.let { "$it · Confirmed" }
            ?: if (summary.isConfirmed) "Confirmed" else "0 · Pending"

    Column(
        modifier =
            Modifier
                .fillMaxWidth()
                .padding(horizontal = 22.dp, vertical = 8.dp),
        verticalArrangement = Arrangement.spacedBy(16.dp),
    ) {
        Row(verticalAlignment = Alignment.Top) {
            TransactionSummaryIcon(summary)

            Spacer(Modifier.width(14.dp))

            Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
                Text(text = summary.label, style = MaterialTheme.typography.titleMedium)
                summary.pendingSend?.let { PendingSendSubtitle(it) }

                Text(
                    text = amount,
                    style = MaterialTheme.typography.titleLarge,
                    fontWeight = FontWeight.SemiBold,
                    modifier = Modifier.padding(top = 6.dp),
                )

                fiat?.let {
                    Text(
                        text = it,
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
            }
        }

        TransactionSummaryField(title = "Date", value = privateShow(summary.date))

        details?.details()?.addressSpacedOut()?.let {
            TransactionSummaryField(title = "Address", value = privateShow(it), monospaced = true)
        }

        TransactionSummaryConfirmations(isConfirmed = summary.isConfirmed, text = confirmations)
    }
}

/** A delayed send's orange clock, or the usual direction icon */
@Composable
private fun TransactionSummaryIcon(summary: TransactionSummary) {
    if (summary.pendingSend?.isDelayed() == true) {
        FeeBumpDelayedIcon(size = 44.dp)
        return
    }

    val icon =
        when {
            !summary.isConfirmed -> Icons.Filled.Schedule
            summary.direction == TransactionDirection.OUTGOING -> Icons.Filled.NorthEast
            else -> Icons.Filled.SouthWest
        }

    // a pending transaction's icon is fainter
    val iconBackgroundAlpha =
        if (summary.isConfirmed) ICON_BACKGROUND_ALPHA else ICON_BACKGROUND_ALPHA * PENDING_ICON_ALPHA

    Box(
        modifier =
            Modifier
                .size(44.dp)
                .clip(RoundedCornerShape(6.dp))
                .background(MaterialTheme.colorScheme.inverseSurface.copy(alpha = iconBackgroundAlpha)),
        contentAlignment = Alignment.Center,
    ) {
        Icon(
            imageVector = icon,
            contentDescription = null,
            tint = MaterialTheme.colorScheme.inverseOnSurface,
            modifier = Modifier.size(22.dp),
        )
    }
}

@Composable
private fun TransactionSummaryField(
    title: String,
    value: String,
    monospaced: Boolean = false,
) {
    Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
        Text(
            text = title,
            style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )

        Text(
            text = value,
            style = MaterialTheme.typography.bodyMedium,
            fontFamily = if (monospaced) FontFamily.Monospace else null,
        )
    }
}

@Composable
private fun TransactionSummaryConfirmations(
    isConfirmed: Boolean,
    text: String,
) {
    Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
        Text(
            text = "Confirmations",
            style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )

        Row(verticalAlignment = Alignment.CenterVertically) {
            Box(
                modifier =
                    Modifier
                        .size(7.dp)
                        .clip(CircleShape)
                        .background(if (isConfirmed) CoveColor.SuccessGreen else SpeedUpOrange),
            )

            Spacer(Modifier.width(6.dp))
            Text(text = text, style = MaterialTheme.typography.bodyMedium)
        }
    }
}

/** Copy Address, enabled once the details hold one, and Copy Transaction ID */
@Composable
private fun TransactionCopyActions(
    summary: TransactionSummary,
    details: TransactionDetailsPresentation?,
    onDismiss: () -> Unit,
) {
    val context = LocalContext.current
    val address = details?.details()?.address()

    TransactionActionRow(
        icon = Icons.Outlined.ContentCopy,
        title = "Copy Address",
        testTag = "transactionMenu.copyAddress",
        enabled = address != null,
        onClick = {
            address?.let { copyToClipboard(context, "Address", it.unformatted(), "Address copied") }
            onDismiss()
        },
    )

    TransactionActionRow(
        icon = Icons.Outlined.Tag,
        title = "Copy Transaction ID",
        testTag = "transactionMenu.copyTxId",
        onClick = {
            copyToClipboard(context, "Transaction ID", summary.txId.toString(), "Transaction ID copied")
            onDismiss()
        },
    )
}

// a menu row needs its icon, copy, tag, action and the two optional states, all distinct
@Composable
@Suppress("LongParameterList")
private fun TransactionActionRow(
    icon: ImageVector,
    title: String,
    testTag: String,
    onClick: () -> Unit,
    subtitle: String? = null,
    enabled: Boolean = true,
) {
    val contentColor = if (enabled) MaterialTheme.colorScheme.onSurface else MaterialTheme.colorScheme.onSurfaceVariant

    Row(
        modifier =
            Modifier
                .fillMaxWidth()
                .clickable(enabled = enabled, onClick = onClick)
                .testTag(testTag)
                .padding(horizontal = 22.dp, vertical = 14.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Surface(shape = CircleShape, color = Color.Transparent) {
            Icon(imageVector = icon, contentDescription = null, tint = contentColor)
        }

        Spacer(Modifier.width(18.dp))

        Column {
            Text(text = title, style = MaterialTheme.typography.bodyLarge, color = contentColor)

            subtitle?.let {
                Text(
                    text = it,
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
        }
    }
}

// android 13 and later confirm a copy themselves, so a toast there would say it twice
private fun copyToClipboard(
    context: Context,
    label: String,
    text: String,
    confirmation: String,
) {
    val clipboard = context.getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager
    clipboard.setPrimaryClip(ClipData.newPlainText(label, text))

    if (Build.VERSION.SDK_INT < Build.VERSION_CODES.TIRAMISU) {
        Toast.makeText(context, confirmation, Toast.LENGTH_SHORT).show()
    }
}
