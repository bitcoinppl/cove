package org.bitcoinppl.cove.wallet_transactions

import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Menu
import androidx.compose.material.icons.filled.MoreVert
import androidx.compose.material.icons.filled.NorthEast
import androidx.compose.material.icons.filled.QrCode2
import androidx.compose.material.icons.filled.SouthWest
import androidx.compose.material.icons.filled.Visibility
import androidx.compose.material.icons.filled.VisibilityOff
import androidx.compose.material3.CenterAlignedTopAppBar
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Scaffold
import androidx.compose.material3.SnackbarHost
import androidx.compose.material3.SnackbarHostState
import androidx.compose.material3.Text
import androidx.compose.material3.TopAppBarDefaults
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.tooling.preview.Preview
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import org.bitcoinppl.cove.R
import org.bitcoinppl.cove.WalletLoadState
import org.bitcoinppl.cove.WalletManager
import org.bitcoinppl.cove.ui.theme.CoveColor
import org.bitcoinppl.cove.views.ImageButton
import org.bitcoinppl.cove_core.Transaction
import org.bitcoinppl.cove_core.TransactionDirection

enum class TransactionType { SENT, RECEIVED }

@Preview(showBackground = true, backgroundColor = 0xFF0D1B2A)
@Composable
private fun WalletTransactionsLightPreview() {
    val snack = remember { SnackbarHostState() }
    WalletTransactionsScreen(
        onBack = {},
        onSend = {},
        onReceive = {},
        onQrCode = {},
        onMore = {},
        isDarkList = false,
        snackbarHostState = snack,
    )
}

@Preview(showBackground = true, backgroundColor = 0xFF0D1B2A)
@Composable
private fun WalletTransactionsDarkPreview() {
    val snack = remember { SnackbarHostState() }
    WalletTransactionsScreen(
        onBack = {},
        onSend = {},
        onReceive = {},
        onQrCode = {},
        onMore = {},
        isDarkList = true,
        initialBalanceHidden = true,
        snackbarHostState = snack,
    )
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun WalletTransactionsScreen(
    onBack: () -> Unit,
    onSend: () -> Unit,
    onReceive: () -> Unit,
    onQrCode: () -> Unit,
    onMore: () -> Unit,
    isDarkList: Boolean,
    manager: WalletManager? = null,
    initialBalanceHidden: Boolean = false,
    usdAmount: String = "$1,351.93",
    satsAmount: String = "1,166,369 SATS",
    walletName: String = "Main",
    snackbarHostState: SnackbarHostState = remember { SnackbarHostState() },
) {
    // extract real data from manager if available
    val actualWalletName = manager?.walletMetadata?.name ?: walletName
    val actualSatsAmount =
        manager?.let {
            val spendable = it.balance.spendable()
            it.displayAmount(spendable, showUnit = true)
        } ?: satsAmount
    val actualUsdAmount =
        manager?.fiatBalance?.let {
            "$${"%.2f".format(it)}"
        } ?: usdAmount
    val transactions =
        when (val state = manager?.loadState) {
            is WalletLoadState.LOADED -> state.txns
            is WalletLoadState.SCANNING -> state.txns
            else -> emptyList()
        }

    val listBg = if (isDarkList) CoveColor.ListBackgroundDark else CoveColor.ListBackgroundLight
    val listCard = if (isDarkList) CoveColor.ListCardDark else CoveColor.ListCardAlternative
    val primaryText = if (isDarkList) CoveColor.TextPrimaryDark else CoveColor.TextPrimaryLight
    val secondaryText = CoveColor.TextSecondary
    val dividerColor = if (isDarkList) CoveColor.DividerDarkAlpha else CoveColor.DividerLightAlpha

    Scaffold(
        containerColor = CoveColor.midnightBlue,
        topBar = {
            CenterAlignedTopAppBar(
                colors =
                    TopAppBarDefaults.centerAlignedTopAppBarColors(
                        containerColor = Color.Transparent,
                        titleContentColor = Color.White,
                        actionIconContentColor = Color.White,
                        navigationIconContentColor = Color.White,
                    ),
                title = {
                    Text(
                        actualWalletName,
                        style = MaterialTheme.typography.titleMedium,
                        maxLines = 1,
                        overflow = TextOverflow.Ellipsis,
                    )
                },
                navigationIcon = {
                    IconButton(onClick = onBack) {
                        Icon(
                            imageVector = Icons.Filled.Menu,
                            contentDescription = "Menu",
                        )
                    }
                },
                actions = {
                    IconButton(onClick = onQrCode) {
                        Icon(
                            imageVector = Icons.Filled.QrCode2,
                            contentDescription = "QR Code",
                        )
                    }
                    IconButton(onClick = onMore) {
                        Icon(
                            imageVector = Icons.Filled.MoreVert,
                            contentDescription = "More",
                        )
                    }
                },
            )
        },
        snackbarHost = { SnackbarHost(snackbarHostState) },
    ) { padding ->
        Box(
            modifier =
                Modifier
                    .fillMaxSize()
                    .padding(padding),
        ) {
            Image(
                painter = painterResource(id = R.drawable.image_chain_code_pattern_horizontal),
                contentDescription = null,
                contentScale = ContentScale.Crop,
                modifier =
                    Modifier
                        .fillMaxWidth()
                        .align(Alignment.TopCenter),
            )

            Column(
                modifier = Modifier.fillMaxSize(),
            ) {
                Column(
                    modifier =
                        Modifier
                            .fillMaxWidth()
                            .padding(start = 16.dp, end = 16.dp, top = 24.dp, bottom = 32.dp),
                    verticalArrangement = Arrangement.spacedBy(24.dp),
                ) {
                    BalanceWidget(
                        usdAmount = actualUsdAmount,
                        satsAmount = actualSatsAmount,
                        hidden = initialBalanceHidden,
                    )

                    Row(horizontalArrangement = Arrangement.spacedBy(16.dp)) {
                        ImageButton(
                            text = stringResource(R.string.btn_send),
                            leading = {
                                Icon(
                                    imageVector = Icons.Filled.NorthEast,
                                    contentDescription = null,
                                )
                            },
                            onClick = onSend,
                            colors =
                                androidx.compose.material3.ButtonDefaults.buttonColors(
                                    containerColor = CoveColor.btnPrimary,
                                    contentColor = CoveColor.midnightBlue,
                                ),
                            modifier = Modifier.weight(1f),
                        )
                        ImageButton(
                            text = stringResource(R.string.btn_receive),
                            leading = {
                                Icon(
                                    imageVector = Icons.Filled.SouthWest,
                                    contentDescription = null,
                                )
                            },
                            onClick = onReceive,
                            colors =
                                androidx.compose.material3.ButtonDefaults.buttonColors(
                                    containerColor = CoveColor.btnPrimary,
                                    contentColor = CoveColor.midnightBlue,
                                ),
                            modifier = Modifier.weight(1f),
                        )
                    }
                }

                Column(
                    modifier =
                        Modifier
                            .fillMaxWidth()
                            .weight(1f) // Fill remaining space
                            .background(listBg)
                            .padding(horizontal = 20.dp, vertical = 16.dp),
                    verticalArrangement = Arrangement.spacedBy(8.dp),
                ) {
                    Text(
                        text = stringResource(R.string.title_transactions),
                        color = secondaryText,
                        fontSize = 16.sp,
                        fontWeight = FontWeight.SemiBold,
                    )

                    // render real transactions or show empty state
                    if (transactions.isEmpty()) {
                        // empty state
                        Box(
                            modifier =
                                Modifier
                                    .fillMaxWidth()
                                    .padding(vertical = 32.dp),
                            contentAlignment = Alignment.Center,
                        ) {
                            Text(
                                text = "No transactions yet",
                                color = secondaryText,
                                fontSize = 14.sp,
                            )
                        }
                    } else {
                        // render transactions dynamically
                        transactions.forEachIndexed { index, txn ->
                            when (txn) {
                                is Transaction.Confirmed -> {
                                    val direction = txn.sentAndReceived().direction()
                                    val txType =
                                        when (direction) {
                                            TransactionDirection.INCOMING -> TransactionType.RECEIVED
                                            TransactionDirection.OUTGOING -> TransactionType.SENT
                                        }

                                    // format amount with manager if available
                                    val formattedAmount =
                                        manager?.let {
                                            val amount = txn.sentAndReceived().amount()
                                            val prefix = if (direction == TransactionDirection.OUTGOING) "-" else ""
                                            prefix + it.displayAmount(amount, showUnit = true)
                                        } ?: txn.sentAndReceived().label()

                                    TransactionWidget(
                                        type = txType,
                                        date = txn.confirmedAtFmt(),
                                        amount = formattedAmount,
                                        balanceAfter = txn.blockHeightFmt(),
                                        listCard = listCard,
                                        primaryText = primaryText,
                                        secondaryText = secondaryText,
                                    )
                                }
                                is Transaction.Unconfirmed -> {
                                    val direction = txn.sentAndReceived().direction()
                                    val txType =
                                        when (direction) {
                                            TransactionDirection.INCOMING -> TransactionType.RECEIVED
                                            TransactionDirection.OUTGOING -> TransactionType.SENT
                                        }

                                    // format amount with manager if available
                                    val formattedAmount =
                                        manager?.let {
                                            val amount = txn.sentAndReceived().amount()
                                            val prefix = if (direction == TransactionDirection.OUTGOING) "-" else ""
                                            prefix + it.displayAmount(amount, showUnit = true)
                                        } ?: txn.sentAndReceived().label()

                                    TransactionWidget(
                                        type = txType,
                                        date = "Pending",
                                        amount = formattedAmount,
                                        balanceAfter = "Unconfirmed",
                                        listCard = listCard,
                                        primaryText = primaryText,
                                        secondaryText = secondaryText,
                                    )
                                }
                            }

                            // add divider between transactions (but not after the last one)
                            if (index < transactions.size - 1) {
                                HorizontalDivider(color = dividerColor, thickness = 0.5.dp)
                            }
                        }
                    }

                    Spacer(Modifier.height(12.dp))
                }
            }
        }
    }
}

@Composable
private fun TransactionWidget(
    type: TransactionType,
    date: String,
    amount: String,
    balanceAfter: String,
    listCard: Color,
    primaryText: Color,
    secondaryText: Color,
) {
    val title =
        stringResource(
            when (type) {
                TransactionType.SENT -> R.string.label_transaction_sent
                TransactionType.RECEIVED -> R.string.label_transaction_received
            },
        )

    Row(
        modifier =
            Modifier
                .fillMaxWidth()
                .padding(vertical = 8.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Box(
            modifier =
                Modifier
                    .size(48.dp)
                    .clip(RoundedCornerShape(8.dp))
                    .background(listCard),
            contentAlignment = Alignment.Center,
        ) {
            Icon(
                imageVector = if (type == TransactionType.SENT) Icons.Filled.NorthEast else Icons.Filled.SouthWest,
                contentDescription = title,
                tint = Color.White,
                modifier = Modifier.size(24.dp),
            )
        }

        Spacer(modifier = Modifier.size(16.dp))

        Column(
            modifier = Modifier.weight(1f),
            verticalArrangement = Arrangement.spacedBy(4.dp),
        ) {
            Text(
                text = title,
                color = primaryText,
                fontSize = 16.sp,
                fontWeight = FontWeight.Medium,
            )
            Text(
                text = date,
                color = secondaryText,
                fontSize = 14.sp,
                fontWeight = FontWeight.Normal,
            )
        }

        Column(horizontalAlignment = Alignment.End) {
            Text(
                text = amount,
                color = if (type == TransactionType.RECEIVED) CoveColor.TransactionReceived else primaryText,
                fontSize = 20.sp,
                fontWeight = FontWeight.Normal,
            )
            Text(
                text = balanceAfter,
                color = secondaryText,
                fontSize = 14.sp,
                fontWeight = FontWeight.Normal,
            )
        }
    }
}

@Composable
private fun BalanceWidget(
    hidden: Boolean,
    usdAmount: String,
    satsAmount: String,
) {
    var isHidden by remember { mutableStateOf(hidden) }

    Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
        Text(
            text = if (isHidden) "$———" else usdAmount,
            color = Color.White.copy(alpha = 0.7f),
            fontSize = 16.sp,
        )

        Row(verticalAlignment = Alignment.CenterVertically) {
            Text(
                text = if (isHidden) "•••••• SATS" else satsAmount,
                color = Color.White,
                fontSize = 36.sp,
                fontWeight = FontWeight.Bold,
                modifier = Modifier.weight(1f),
            )
            IconButton(onClick = { isHidden = !isHidden }) {
                Icon(
                    imageVector = if (isHidden) Icons.Filled.VisibilityOff else Icons.Filled.Visibility,
                    contentDescription = if (isHidden) "Show" else "Hide",
                    tint = Color.White.copy(alpha = 0.7f),
                )
            }
        }
    }
}
