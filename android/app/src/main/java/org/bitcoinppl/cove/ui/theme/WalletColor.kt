package org.bitcoinppl.cove.ui.theme

import androidx.compose.ui.graphics.Color
//TODO:Check the ids and the Actual data from COVE Rust LIB
enum class WalletColor(val id: Int, val color: Color) {
    LIGHT_ORANGE(1, walletColorLightOrange),
    LIGHT_BLUE(2, walletColorLightBlue),
    DARK_BLUE(3, walletColorDarkBlue),
    LIGHT_RED(4, walletColorLightRed),
    YELLOW(5, walletColorYellow),
    LIGHT_GREEN(6, walletColorLightGreen),
    BLUE(7, walletColorBlue),
    GREEN(8, walletGreen),
    ORANGE(9, walletColorOrange),
    PURPLE(10, walletColorPurple);

    companion object {
        fun getWalletColorById(id: Int): WalletColor? =
            entries.find { it.id == id }
    }
}