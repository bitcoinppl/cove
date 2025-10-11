package org.bitcoinppl.cove.settings

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.asPaddingValues
import androidx.compose.foundation.layout.aspectRatio
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.safeDrawing
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.lazy.grid.GridCells
import androidx.compose.foundation.lazy.grid.LazyVerticalGrid
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.automirrored.filled.KeyboardArrowRight
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalWindowInfo
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.tooling.preview.Preview
import androidx.compose.ui.unit.dp
import org.bitcoinppl.cove.R
import org.bitcoinppl.cove.ui.theme.WalletColor
import org.bitcoinppl.cove.views.CardItem
import org.bitcoinppl.cove.views.ClickableInfoRow
import org.bitcoinppl.cove.views.CustomSpacer
import org.bitcoinppl.cove.views.InfoRow
import org.bitcoinppl.cove.views.SwitchRow

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun WalletSettingsScreen(wallet: TempWalletMock) {
    Scaffold(
        modifier = Modifier
            .fillMaxSize()
            .padding(WindowInsets.safeDrawing.asPaddingValues()),
        topBar = @Composable {
            TopAppBar(
                title = {
                    Box(
                        modifier = Modifier.fillMaxSize(),
                        contentAlignment = Alignment.Center
                    ) {
                        Text(
                            style = MaterialTheme.typography.bodyLarge,
                            text = wallet.settings.name,
                            textAlign = TextAlign.Center
                        )
                    }
                },
                navigationIcon = {
                    IconButton(onClick = {
                        //TODO:navigate back to Settings
                    }) {
                        Icon(Icons.AutoMirrored.Default.ArrowBack, contentDescription = "Back")
                    }
                },
                actions = { },
                modifier = Modifier.height(56.dp)
            )
        },
        content = { paddingValues ->
            Column(
                modifier = Modifier
                    .fillMaxSize()
                    .verticalScroll(rememberScrollState())
                    .padding(paddingValues)
                    .padding(horizontal = 16.dp)
            ) {
                CardItem(stringResource(R.string.title_wallet_information), allCaps = true) {
                    Column(
                        modifier = Modifier
                            .padding(vertical = 8.dp)
                            .padding(start = 8.dp),
                    ) {
                        InfoRow(stringResource(R.string.label_wallet_network), wallet.networkName)
                        ListSpacer()
                        InfoRow(
                            stringResource(R.string.label_wallet_fingerprint),
                            wallet.fingerPrint
                        )
                        ListSpacer()
                        InfoRow(stringResource(R.string.label_wallet_type), wallet.walletType)
                    }
                }
                CardItem(title = stringResource(R.string.title_wallet_settings), allCaps = true) {
                    Column(
                        modifier = Modifier
                            .padding(vertical = 8.dp)
                            .padding(start = 8.dp),
                    ) {
                        ClickableInfoRow(
                            stringResource(R.string.label_wallet_name),
                            wallet.settings.name,
                            Icons.AutoMirrored.Default.KeyboardArrowRight
                        ) {
                            //TODO:NAME CHANGE?
                        }
                        ListSpacer()
                        WalletColorSelector(wallet.settings.colorId)
                        ListSpacer()
                        SwitchRow(
                            stringResource(R.string.label_wallet_show_transaction_labels),
                            wallet.settings.showLabels
                        ) { isChecked ->
                            //TODO: changeSettings of wallet
                        }
                    }
                }

                CardItem(
                    title = stringResource(R.string.title_wallet_danger_zone),
                    allCaps = true,
                    titleColor = Color.Black
                ) {
                    Column(
                        modifier = Modifier
                            .padding(vertical = 8.dp)
                            .padding(start = 8.dp),
                    ) {
                        Text(
                            modifier = Modifier
                                .fillMaxWidth()
                                .padding(all = 8.dp)
                                .clickable(true) {
                                    //TODO:SHOW SECRETS wallet flow
                                },
                            text = stringResource(R.string.label_wallet_view_secrets),
                            style = MaterialTheme.typography.bodyMedium,
                            textAlign = TextAlign.Start,
                        )
                        ListSpacer()
                        Text(
                            modifier = Modifier
                                .fillMaxWidth()
                                .padding(all = 8.dp)
                                .clickable(true) {
                                    //TODO:DELETE wallet flow
                                },
                            text = stringResource(R.string.label_wallet_delete),
                            style = MaterialTheme.typography.bodyLarge,
                            color = MaterialTheme.colorScheme.error,
                            textAlign = TextAlign.Start,
                        )
                    }
                }
            }
        }
    )
}

@Preview
@Composable
fun WalletSettingsScreenPreview() {
    WalletSettingsScreen(
        TempWalletMock(
            id = "Id_!@#@!", networkName = "Signet", fingerPrint = "AsSdf322", walletType = "HOT",
            WalletSettings(name = "MyWallet", WalletColor.LIGHT_RED.id, true)
        )
    )
}

@Preview
@Composable
fun WalletColorSelectorPreview() {
    WalletColorSelector(WalletColor.LIGHT_ORANGE.id)
}

@Composable
private fun WalletColorSelector(selectedColorId: Int) {
    var selectedColor by remember {
        mutableStateOf(
            WalletColor.getWalletColorById(selectedColorId) ?: WalletColor.LIGHT_BLUE
        )
    }

    val windowInfo = LocalWindowInfo.current
    val containerSize = windowInfo.containerSize.width

    Column(
        Modifier
            .fillMaxWidth()
            .padding(8.dp)
    ) {
        Text(
            modifier = Modifier
                .fillMaxWidth(),
            text = stringResource(R.string.label_wallet_color),
            style = MaterialTheme.typography.bodyLarge,
            textAlign = TextAlign.Start,
        )
        Row(
            modifier = Modifier
                .fillMaxWidth(), verticalAlignment = Alignment.CenterVertically
        ) {
            Box(
                Modifier
                    .aspectRatio(1f)
                    .background(
                        color = selectedColor.color,
                        shape = RoundedCornerShape(8.dp)
                    ).weight(1f),
            )


            LazyVerticalGrid(
                columns = GridCells.Fixed(5), // 5 per row, adjust as needed
                userScrollEnabled = false,

                modifier = Modifier
                    .fillMaxWidth()
                    .heightIn(max = 200.dp)
                    .padding(4.dp)
                    .weight(3f),
                contentPadding = PaddingValues(2.dp)
            ) {
                items(WalletColor.entries.size) { index ->
                    val walletColor = WalletColor.entries[index]

                    Box(
                        modifier = Modifier
                            .padding(4.dp)
                            .aspectRatio(1f)
                            .size(48.dp) // circle size
                            .clickable { selectedColor = walletColor }
                    ) {
                        // If selected → border first
                        if (walletColor == selectedColor) {
                            Box(
                                modifier = Modifier
                                    .matchParentSize()
                                    .padding(3.dp) // creates space between border and circle
                                    .border(
                                        width = 3.dp,
                                        color = MaterialTheme.colorScheme.primary,
                                        shape = CircleShape
                                    )
                            )
                        }

                        // color circle
                        Box(
                            modifier = Modifier
                                .fillMaxSize()
                                .background(walletColor.color, CircleShape)
                        )
                    }
                }
            }
        }
    }
}


@Composable
private fun ListSpacer() {
    CustomSpacer(height = 8.dp, paddingValues = PaddingValues(start = 8.dp))
}

//TODO:Remove and change to a real Wallet from CoveLib
data class TempWalletMock(
    val id: String,
    val networkName: String,
    val fingerPrint: String,
    val walletType: String,
    val settings: WalletSettings
)

//TODO:Remove and change to a real Wallet from CoveLib
data class WalletSettings(
    val name: String,
    val colorId: Int,
    val showLabels: Boolean
)
