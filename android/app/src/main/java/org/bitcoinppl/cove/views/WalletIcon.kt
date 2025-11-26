package org.bitcoinppl.cove.views

import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.AccountBalanceWallet
import androidx.compose.runtime.Composable
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import org.bitcoinppl.cove.utils.isLightColor
import org.bitcoinppl.cove.utils.toComposeColor
import org.bitcoinppl.cove_core.WalletColor
import org.bitcoinppl.cove_core.WalletMetadata

@Composable
fun WalletIcon(
    wallet: WalletMetadata,
    size: Dp = 40.dp,
    cornerRadius: Dp = 8.dp,
) {
    val iconTint = if (wallet.color.isLightColor()) Color.Black else Color.White
    RoundRectImage(
        size = size,
        backgroundColor = wallet.color.toComposeColor(),
        painter =
            androidx.compose.ui.graphics.vector
                .rememberVectorPainter(Icons.Default.AccountBalanceWallet),
        contentDescription = null,
        cornerRadius = cornerRadius,
        imageTint = iconTint,
    )
}

@Composable
fun WalletIcon(
    walletColor: WalletColor,
    size: Dp = 40.dp,
    cornerRadius: Dp = 8.dp,
) {
    val iconTint = if (walletColor.isLightColor()) Color.Black else Color.White
    RoundRectImage(
        size = size,
        backgroundColor = walletColor.toComposeColor(),
        painter =
            androidx.compose.ui.graphics.vector
                .rememberVectorPainter(Icons.Default.AccountBalanceWallet),
        contentDescription = null,
        cornerRadius = cornerRadius,
        imageTint = iconTint,
    )
}
