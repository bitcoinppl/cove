package org.bitcoinppl.cove.sidebar

import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.asPaddingValues
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.safeDrawing
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Add
import androidx.compose.material.icons.filled.Nfc
import androidx.compose.material.icons.filled.Settings
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import org.bitcoinppl.cove.AppManager
import org.bitcoinppl.cove.R
import org.bitcoinppl.cove.ui.theme.CoveColor
import org.bitcoinppl.cove_core.Route
import org.bitcoinppl.cove_core.RouteFactory
import org.bitcoinppl.cove_core.SettingsRoute
import org.bitcoinppl.cove_core.WalletColor
import org.bitcoinppl.cove_core.WalletMetadata

@Composable
fun SidebarView(
    app: AppManager,
    modifier: Modifier = Modifier,
) {
    val coroutineScope = rememberCoroutineScope()

    Column(
        modifier =
            modifier
                .width(280.dp)
                .fillMaxHeight()
                .background(CoveColor.midnightBlue)
                .padding(WindowInsets.safeDrawing.asPaddingValues())
                .padding(horizontal = 20.dp),
        verticalArrangement = Arrangement.spacedBy(0.dp),
    ) {
        // header with icon and NFC button
        Row(
            modifier = Modifier.fillMaxWidth(),
            horizontalArrangement = Arrangement.SpaceBetween,
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Image(
                painter = painterResource(id = R.drawable.cove_logo),
                contentDescription = "Cove",
                modifier =
                    Modifier
                        .size(65.dp)
                        .clip(CircleShape),
            )

            IconButton(
                onClick = {
                    app.closeSidebarAndNavigate {
                        app.scanNfc()
                    }
                },
            ) {
                Icon(
                    imageVector = Icons.Default.Nfc,
                    contentDescription = "NFC Scan",
                    tint = Color.White,
                    modifier = Modifier.size(24.dp),
                )
            }
        }

        Spacer(modifier = Modifier.height(22.dp))

        HorizontalDivider(
            color = Color.White.copy(alpha = 0.5f),
            thickness = 1.dp,
        )

        Spacer(modifier = Modifier.height(22.dp))

        // my wallets header
        Text(
            text = "My Wallets",
            color = Color.White,
            fontSize = 17.sp,
            fontWeight = FontWeight.Medium,
        )

        Spacer(modifier = Modifier.height(12.dp))

        // wallet list
        LazyColumn(
            modifier = Modifier.weight(1f),
            verticalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            items(app.wallets) { wallet ->
                WalletItem(
                    wallet = wallet,
                    onClick = {
                        app.closeSidebarAndNavigate {
                            app.rust.selectWallet(wallet.id)
                        }
                    },
                )
            }
        }

        Spacer(modifier = Modifier.height(16.dp))

        HorizontalDivider(
            color = CoveColor.coveLightGray.copy(alpha = 0.5f),
            thickness = 1.dp,
        )

        Spacer(modifier = Modifier.height(32.dp))

        // add wallet button
        Row(
            modifier =
                Modifier
                    .fillMaxWidth()
                    .clickable {
                        app.isSidebarVisible = false
                        coroutineScope.launch {
                            delay(300)
                            if (app.wallets.isEmpty()) {
                                app.resetRoute(RouteFactory().newWalletSelect())
                            } else {
                                app.pushRoute(RouteFactory().newWalletSelect())
                            }
                        }
                    }.padding(vertical = 8.dp),
            horizontalArrangement = Arrangement.spacedBy(20.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Icon(
                imageVector = Icons.Default.Add,
                contentDescription = null,
                tint = Color.White,
                modifier = Modifier.size(24.dp),
            )
            Text(
                text = "Add Wallet",
                color = Color.White,
                fontSize = 17.sp,
            )
        }

        Spacer(modifier = Modifier.height(22.dp))

        // settings button
        Row(
            modifier =
                Modifier
                    .fillMaxWidth()
                    .clickable {
                        app.closeSidebarAndNavigate {
                            app.pushRoute(Route.Settings(SettingsRoute.Main))
                        }
                    }.padding(vertical = 8.dp),
            horizontalArrangement = Arrangement.spacedBy(20.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Icon(
                imageVector = Icons.Default.Settings,
                contentDescription = null,
                tint = Color.White,
                modifier = Modifier.size(24.dp),
            )
            Text(
                text = "Settings",
                color = Color.White,
                fontSize = 17.sp,
            )
        }
    }
}

@Composable
private fun WalletItem(
    wallet: WalletMetadata,
    onClick: () -> Unit,
) {
    Row(
        modifier =
            Modifier
                .fillMaxWidth()
                .clip(RoundedCornerShape(10.dp))
                .background(CoveColor.coveLightGray.copy(alpha = 0.06f))
                .clickable(onClick = onClick)
                .padding(16.dp),
        horizontalArrangement = Arrangement.spacedBy(10.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        // color indicator
        Box(
            modifier =
                Modifier
                    .size(8.dp)
                    .clip(CircleShape)
                    .background(wallet.color.toComposeColor()),
        )

        // wallet name
        AutoSizeText(
            text = wallet.name ?: "Wallet",
            color = Color.White,
            maxFontSize = 17.sp,
            minimumScaleFactor = 0.90f,
            modifier = Modifier.weight(1f),
        )
    }
}

// convert wallet color to compose color
private fun WalletColor.toComposeColor(): Color =
    when (this) {
        is WalletColor.Red -> CoveColor.pastelRed
        is WalletColor.Blue -> CoveColor.pastelBlue
        is WalletColor.Green -> CoveColor.walletGreen
        is WalletColor.Yellow -> CoveColor.pastelYellow
        is WalletColor.Orange -> CoveColor.beige
        is WalletColor.Purple -> CoveColor.walletColorPurple
        is WalletColor.Pink -> CoveColor.walletColorLightRed
        is WalletColor.CoolGray -> CoveColor.almostGray
        is WalletColor.WBeige -> CoveColor.beige
        is WalletColor.WPastelBlue -> CoveColor.pastelBlue
        is WalletColor.WPastelNavy -> CoveColor.pastelNavy
        is WalletColor.WPastelRed -> CoveColor.pastelRed
        is WalletColor.WPastelYellow -> CoveColor.pastelYellow
        is WalletColor.WLightMint -> CoveColor.lightMint
        is WalletColor.WPastelTeal -> CoveColor.pastelTeal
        is WalletColor.WLightPastelYellow -> CoveColor.lightPastelYellow
        is WalletColor.WAlmostGray -> CoveColor.almostGray
        is WalletColor.WAlmostWhite -> CoveColor.almostWhite
        is WalletColor.Custom -> Color.Gray
    }
