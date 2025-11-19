package org.bitcoinppl.cove.settings

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.asPaddingValues
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.safeDrawing
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
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
import org.bitcoinppl.cove.views.SettingsItem

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun SettingsScreen(
    app: org.bitcoinppl.cove.AppManager,
    modifier: Modifier = Modifier,
) {
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
                            text = stringResource(R.string.title_settings),
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
            Box(
                modifier = Modifier.fillMaxSize(),
            ) {
                Column(
                    modifier =
                        Modifier
                            .fillMaxSize()
                            .verticalScroll(rememberScrollState())
                            .padding(paddingValues)
                            .padding(horizontal = 16.dp),
                ) {
                    CardItem(stringResource(R.string.title_settings_general)) {
                        Column(
                            modifier =
                                Modifier
                                    .padding(vertical = 8.dp)
                                    .padding(start = 8.dp),
                        ) {
                            SettingsItem(
                                stringResource(R.string.title_settings_network),
                                iconResId = R.drawable.icon_network,
                                onClick = {
                                    app.pushRoute(org.bitcoinppl.cove_core.Route.Settings(org.bitcoinppl.cove_core.SettingsRoute.Network))
                                },
                            )
                            Spacer()
                            SettingsItem(
                                stringResource(R.string.title_settings_appearance),
                                iconResId = R.drawable.icon_appearance,
                                onClick = {
                                    app.pushRoute(org.bitcoinppl.cove_core.Route.Settings(org.bitcoinppl.cove_core.SettingsRoute.Appearance))
                                },
                            )
                            Spacer()
                            SettingsItem(
                                stringResource(R.string.title_settings_node),
                                iconResId = R.drawable.icon_node,
                                onClick = {
                                    app.pushRoute(org.bitcoinppl.cove_core.Route.Settings(org.bitcoinppl.cove_core.SettingsRoute.Node))
                                },
                            )
                            Spacer()
                            SettingsItem(
                                stringResource(R.string.title_settings_currency),
                                iconResId = R.drawable.icon_currency,
                                onClick = {
                                    app.pushRoute(org.bitcoinppl.cove_core.Route.Settings(org.bitcoinppl.cove_core.SettingsRoute.FiatCurrency))
                                },
                            )
                        }
                    }
                }

                Column(
                    modifier =
                        Modifier
                            .align(Alignment.BottomCenter)
                            .padding(bottom = 16.dp),
                    horizontalAlignment = Alignment.CenterHorizontally,
                ) {
                    Row(horizontalArrangement = Arrangement.Center) {
                        Text(
                            text = app.rust.debugOrRelease(),
                            style = MaterialTheme.typography.labelSmall,
                            color = MaterialTheme.colorScheme.onSurfaceVariant.copy(alpha = 0.6f),
                        )
                    }
                    Row(horizontalArrangement = Arrangement.Center) {
                        Text(
                            text = app.fullVersionId,
                            style = MaterialTheme.typography.labelSmall,
                            color = MaterialTheme.colorScheme.onSurfaceVariant.copy(alpha = 0.6f),
                        )
                    }
                    Row(horizontalArrangement = Arrangement.Center) {
                        Text(
                            text = "feedback@covebitcoinwallet.com",
                            style = MaterialTheme.typography.labelSmall,
                            color = MaterialTheme.colorScheme.onSurfaceVariant.copy(alpha = 0.6f),
                        )
                    }
                }
            }
        },
    )
}

@Composable
private fun Spacer() {
    CustomSpacer(paddingValues = PaddingValues(start = 56.dp))
}
