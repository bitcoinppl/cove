package org.bitcoinppl.cove.settings

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.asPaddingValues
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.safeDrawing
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.filled.Check
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import org.bitcoinppl.cove.R
import org.bitcoinppl.cove.views.CardItem
import org.bitcoinppl.cove.views.CustomSpacer
import org.bitcoinppl.cove_core.AppAction
import org.bitcoinppl.cove_core.FiatCurrency
import org.bitcoinppl.cove_core.allFiatCurrencies
import org.bitcoinppl.cove_core.fiatCurrencyEmoji
import org.bitcoinppl.cove_core.fiatCurrencyToString

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun FiatCurrencySettingsScreen(
    app: org.bitcoinppl.cove.AppManager,
    modifier: Modifier = Modifier,
) {
    val fiatCurrencies = allFiatCurrencies()
    val selectedFiatCurrency = app.selectedFiatCurrency

    Scaffold(
        modifier =
            modifier
                .fillMaxSize()
                .padding(WindowInsets.safeDrawing.asPaddingValues()),
        topBar = @Composable {
            TopAppBar(
                title = {
                    Box(
                        modifier = Modifier.fillMaxSize(),
                        contentAlignment = Alignment.Center,
                    ) {
                        Text(
                            style = MaterialTheme.typography.bodyLarge,
                            text = stringResource(R.string.title_settings_currency),
                            textAlign = TextAlign.Center,
                        )
                    }
                },
                navigationIcon = {
                    IconButton(onClick = { app.popRoute() }) {
                        Icon(Icons.AutoMirrored.Default.ArrowBack, contentDescription = "Back")
                    }
                },
                actions = { },
                modifier = Modifier.height(56.dp),
            )
        },
        content = { paddingValues ->
            Column(
                modifier =
                    Modifier
                        .fillMaxSize()
                        .verticalScroll(rememberScrollState())
                        .padding(paddingValues)
                        .padding(horizontal = 16.dp),
            ) {
                CardItem(stringResource(R.string.title_settings_currency)) {
                    Column(
                        modifier =
                            Modifier
                                .padding(vertical = 8.dp),
                    ) {
                        fiatCurrencies.forEachIndexed { index, fiatCurrency ->
                            FiatCurrencyRow(
                                fiatCurrency = fiatCurrency,
                                isSelected = fiatCurrency == selectedFiatCurrency,
                                onClick = {
                                    app.dispatch(AppAction.ChangeFiatCurrency(fiatCurrency))
                                },
                            )

                            // add divider between items, but not after the last one
                            if (index < fiatCurrencies.size - 1) {
                                CustomSpacer(paddingValues = PaddingValues(start = 16.dp))
                            }
                        }
                    }
                }
            }
        },
    )
}

@Composable
private fun FiatCurrencyRow(
    fiatCurrency: FiatCurrency,
    isSelected: Boolean,
    onClick: () -> Unit,
) {
    Row(
        modifier =
            Modifier
                .fillMaxWidth()
                .clickable(onClick = onClick)
                .padding(horizontal = 16.dp, vertical = 12.dp),
        horizontalArrangement = Arrangement.SpaceBetween,
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Text(
            text = "${fiatCurrencyEmoji(fiatCurrency)} ${fiatCurrencyToString(fiatCurrency)}",
            style = MaterialTheme.typography.bodyLarge,
        )

        if (isSelected) {
            Icon(
                imageVector = Icons.Default.Check,
                contentDescription = "Selected",
                tint = MaterialTheme.colorScheme.primary,
            )
        }
    }
}
