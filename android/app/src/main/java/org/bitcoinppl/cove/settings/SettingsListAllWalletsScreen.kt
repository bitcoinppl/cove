package org.bitcoinppl.cove.settings

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.asPaddingValues
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.safeDrawing
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.automirrored.filled.KeyboardArrowRight
import androidx.compose.material.icons.filled.AccountBalanceWallet
import androidx.compose.material.icons.filled.Search
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import org.bitcoinppl.cove.utils.toComposeColor
import org.bitcoinppl.cove.views.RoundRectImage
import org.bitcoinppl.cove_core.Database
import org.bitcoinppl.cove_core.Route
import org.bitcoinppl.cove_core.SettingsRoute
import org.bitcoinppl.cove_core.WalletMetadata
import org.bitcoinppl.cove_core.WalletSettingsRoute

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun SettingsListAllWalletsScreen(
    app: org.bitcoinppl.cove.AppManager,
    modifier: Modifier = Modifier,
) {
    var allWallets by remember { mutableStateOf<List<WalletMetadata>>(emptyList()) }
    var searchText by remember { mutableStateOf("") }

    // fetch all wallets on screen appear
    LaunchedEffect(Unit) {
        allWallets = Database().wallets().allSortedActive()
    }

    // filter wallets based on search text
    val filteredWallets =
        remember(allWallets, searchText) {
            if (searchText.isEmpty()) {
                allWallets
            } else {
                allWallets.filter { wallet ->
                    wallet.name.contains(searchText, ignoreCase = true)
                }
            }
        }

    Scaffold(
        modifier =
            modifier
                .fillMaxSize()
                .padding(WindowInsets.safeDrawing.asPaddingValues()),
        topBar = {
            TopAppBar(
                title = {
                    Box(
                        modifier = Modifier.fillMaxSize(),
                        contentAlignment = Alignment.Center,
                    ) {
                        Text(
                            style = MaterialTheme.typography.bodyLarge,
                            text = "All Wallets",
                            textAlign = TextAlign.Center,
                        )
                    }
                },
                navigationIcon = {
                    IconButton(onClick = { app.popRoute() }) {
                        Icon(Icons.AutoMirrored.Default.ArrowBack, contentDescription = "Back")
                    }
                },
                actions = {},
                modifier = Modifier.height(56.dp),
            )
        },
        content = { paddingValues ->
            Column(
                modifier =
                    Modifier
                        .fillMaxSize()
                        .padding(paddingValues)
                        .padding(horizontal = 16.dp),
            ) {
                // search bar
                SearchBar(
                    query = searchText,
                    onQueryChange = { searchText = it },
                )

                Spacer(modifier = Modifier.height(16.dp))

                // wallet list
                if (filteredWallets.isEmpty()) {
                    // empty state
                    Box(
                        modifier = Modifier.fillMaxSize(),
                        contentAlignment = Alignment.Center,
                    ) {
                        Text(
                            text = if (searchText.isEmpty()) "No wallets found" else "No wallets match your search",
                            style = MaterialTheme.typography.bodyLarge,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                    }
                } else {
                    LazyColumn(
                        modifier = Modifier.fillMaxSize(),
                    ) {
                        items(filteredWallets, key = { it.id.toString() }) { wallet ->
                            WalletRow(
                                wallet = wallet,
                                onClick = {
                                    app.pushRoute(
                                        Route.Settings(
                                            SettingsRoute.Wallet(
                                                id = wallet.id,
                                                route = WalletSettingsRoute.MAIN,
                                            ),
                                        ),
                                    )
                                },
                            )
                        }
                    }
                }
            }
        },
    )
}

@Composable
private fun SearchBar(
    query: String,
    onQueryChange: (String) -> Unit,
) {
    val bg = MaterialTheme.colorScheme.surfaceVariant
    Row(
        modifier =
            Modifier
                .fillMaxWidth()
                .clip(RoundedCornerShape(10.dp))
                .background(bg)
                .padding(horizontal = 12.dp, vertical = 10.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Icon(
            imageVector = Icons.Filled.Search,
            contentDescription = null,
            tint = MaterialTheme.colorScheme.outline,
            modifier = Modifier.size(20.dp),
        )
        Spacer(Modifier.width(8.dp))
        BasicTextField(
            value = query,
            onValueChange = onQueryChange,
            textStyle =
                MaterialTheme.typography.bodyMedium.copy(
                    color = MaterialTheme.colorScheme.onSurface,
                    fontSize = 17.sp,
                ),
            singleLine = true,
            modifier = Modifier.weight(1f),
            decorationBox = { innerTextField ->
                if (query.isEmpty()) {
                    Text(
                        "Search Wallets",
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                        fontSize = 17.sp,
                    )
                }
                innerTextField()
            },
        )
    }
}

@Composable
private fun WalletRow(
    wallet: WalletMetadata,
    onClick: () -> Unit,
) {
    Row(
        modifier =
            Modifier
                .fillMaxWidth()
                .clickable(onClick = onClick)
                .padding(vertical = 12.dp, horizontal = 8.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        // wallet icon with colored background
        RoundRectImage(
            size = 40.dp,
            backgroundColor = wallet.color.toComposeColor(),
            painter =
                androidx.compose.ui.graphics.vector
                    .rememberVectorPainter(Icons.Default.AccountBalanceWallet),
            contentDescription = null,
            cornerRadius = 8.dp,
            imageTint = Color.White,
        )

        // wallet name
        Text(
            text = wallet.name,
            style = MaterialTheme.typography.bodyLarge,
            modifier =
                Modifier
                    .weight(1f)
                    .padding(horizontal = 12.dp),
        )

        // chevron arrow
        Icon(
            modifier = Modifier.size(40.dp),
            imageVector = Icons.AutoMirrored.Default.KeyboardArrowRight,
            contentDescription = "Go",
            tint = MaterialTheme.colorScheme.onSurfaceVariant,
        )
    }
}
