package org.bitcoinppl.cove.utils

import androidx.compose.ui.graphics.Color
import org.bitcoinppl.cove_core.*
import org.bitcoinppl.cove_core.types.*

/**
 * extension to convert FFI WalletColor to Compose Color
 */
fun WalletColor.toComposeColor(): Color =
    when (this) {
        is WalletColor.Red -> Color(0xFFFF3B30)
        is WalletColor.Orange -> Color(0xFFFF9500)
        is WalletColor.Yellow -> Color(0xFFFFCC00)
        is WalletColor.Green -> Color(0xFF34C759)
        is WalletColor.Blue -> Color(0xFF007AFF)
        is WalletColor.Purple -> Color(0xFFAF52DE)
        is WalletColor.Pink -> Color(0xFFFF2D55)
        is WalletColor.CoolGray -> Color(0xFF8E8E93)
        is WalletColor.Custom ->
            Color(
                red = this.r.toInt(),
                green = this.g.toInt(),
                blue = this.b.toInt(),
            )
        is WalletColor.WAlmostGray -> Color(0xFF9E9E9E)
        is WalletColor.WAlmostWhite -> Color(0xFFF5F5F5)
        is WalletColor.WBeige -> Color(0xFFD7CCC8)
        is WalletColor.WPastelBlue -> Color(0xFFBBDEFB)
        is WalletColor.WPastelNavy -> Color(0xFF5C6BC0)
        is WalletColor.WPastelRed -> Color(0xFFFFCDD2)
        is WalletColor.WPastelYellow -> Color(0xFFFFF9C4)
        is WalletColor.WLightMint -> Color(0xFFE0F2F1)
        is WalletColor.WPastelTeal -> Color(0xFFB2DFDB)
        is WalletColor.WLightPastelYellow -> Color(0xFFFFFDE7)
    }

fun WalletColor.isLightColor(): Boolean =
    when (this) {
        is WalletColor.WAlmostWhite -> true
        is WalletColor.WLightMint -> true
        else -> false
    }
