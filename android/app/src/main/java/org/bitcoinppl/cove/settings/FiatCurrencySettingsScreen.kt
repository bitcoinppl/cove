package org.bitcoinppl.cove.settings

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material3.CenterAlignedTopAppBar
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.TopAppBarDefaults
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import org.bitcoinppl.cove.managers.AppManager
import uniffi.cove_core.AppAction
import uniffi.cove_core.FiatCurrency

/**
 * fiat currency settings screen
 * allows user to select preferred fiat currency for display
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun FiatCurrencySettingsScreen(
    app: AppManager,
    modifier: Modifier = Modifier
) {
    val currentCurrency = app.selectedFiatCurrency

    Scaffold(
        containerColor = Color.Transparent,
        topBar = {
            CenterAlignedTopAppBar(
                colors = TopAppBarDefaults.centerAlignedTopAppBarColors(
                    containerColor = Color.Transparent
                ),
                title = {
                    Text(
                        "Currency",
                        style = MaterialTheme.typography.titleMedium,
                        fontWeight = FontWeight.SemiBold,
                        maxLines = 1,
                        overflow = TextOverflow.Ellipsis
                    )
                },
                navigationIcon = {
                    IconButton(onClick = { app.popRoute() }) {
                        Icon(
                            imageVector = Icons.AutoMirrored.Default.ArrowBack,
                            contentDescription = "Back"
                        )
                    }
                }
            )
        }
    ) { padding ->
        Box(
            modifier = modifier
                .fillMaxSize()
                .padding(padding)
        ) {
            SettingsPicker(
                items = FiatCurrency.entries,
                selectedItem = currentCurrency,
                onItemSelected = { currency ->
                    app.dispatch(AppAction.ChangeFiatCurrency(currency))
                },
                itemLabel = { currency ->
                    // format: "USD - US Dollar"
                    "${currency.name} - ${currency.symbol}"
                },
                itemSymbol = { "💵" }
            )
        }
    }
}
