package org.bitcoinppl.cove.utils

import androidx.compose.ui.graphics.Color

/**
 * extension to convert FFI WalletColor to Compose Color
 */
fun org.bitcoinppl.cove.WalletColor.toComposeColor(): Color {
    return when (this) {
        org.bitcoinppl.cove.WalletColor.RED -> Color(0xFFFF3B30)
        org.bitcoinppl.cove.WalletColor.ORANGE -> Color(0xFFFF9500)
        org.bitcoinppl.cove.WalletColor.YELLOW -> Color(0xFFFFCC00)
        org.bitcoinppl.cove.WalletColor.GREEN -> Color(0xFF34C759)
        org.bitcoinppl.cove.WalletColor.MINT -> Color(0xFF00C7BE)
        org.bitcoinppl.cove.WalletColor.TEAL -> Color(0xFF30B0C7)
        org.bitcoinppl.cove.WalletColor.CYAN -> Color(0xFF32ADE6)
        org.bitcoinppl.cove.WalletColor.BLUE -> Color(0xFF007AFF)
        org.bitcoinppl.cove.WalletColor.INDIGO -> Color(0xFF5856D6)
        org.bitcoinppl.cove.WalletColor.PURPLE -> Color(0xFFAF52DE)
        org.bitcoinppl.cove.WalletColor.PINK -> Color(0xFFFF2D55)
        org.bitcoinppl.cove.WalletColor.BROWN -> Color(0xFFA2845E)
        org.bitcoinppl.cove.WalletColor.GRAY -> Color(0xFF8E8E93)
    }
}
