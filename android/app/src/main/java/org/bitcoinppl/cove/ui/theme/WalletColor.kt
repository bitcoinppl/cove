package org.bitcoinppl.cove.ui.theme

import androidx.compose.ui.graphics.Color

//TODO:Check the ids and the Actual data from COVE Rust LIB
enum class WalletColor(val id: Int, val color: Color) {
    LIGHT_ORANGE(1, CoveColor.walletColorLightOrange),
    LIGHT_BLUE(2, CoveColor.walletColorLightBlue),
    DARK_BLUE(3, CoveColor.walletColorDarkBlue),
    LIGHT_RED(4, CoveColor.walletColorLightRed),
    YELLOW(5, CoveColor.walletColorYellow),
    LIGHT_GREEN(6, CoveColor.walletColorLightGreen),
    BLUE(7, CoveColor.walletColorBlue),
    GREEN(8, CoveColor.walletGreen),
    ORANGE(9, CoveColor.walletColorOrange),
    PURPLE(10, CoveColor.walletColorPurple);

    companion object {
        fun getWalletColorById(id: Int): WalletColor? =
            entries.find { it.id == id }
    }
}
