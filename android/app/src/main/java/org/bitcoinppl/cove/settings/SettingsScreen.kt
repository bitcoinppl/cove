package org.bitcoinppl.cove.settings

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
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
import androidx.compose.ui.tooling.preview.Preview
import androidx.compose.ui.unit.dp
import org.bitcoinppl.cove.R
import org.bitcoinppl.cove.views.CardItem
import org.bitcoinppl.cove.views.CustomSpacer
import org.bitcoinppl.cove.views.SettingsItem

@OptIn(ExperimentalMaterial3Api::class)
@Preview
@Composable
fun SettingsScreen() {
    Scaffold(
        modifier =
            Modifier
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
                    IconButton(onClick = {
                        // TODO:navigate back
                    }) {
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
                                // TODO:Navigate to general Settings screen
                            },
                        )
                        Spacer()
                        SettingsItem(
                            stringResource(R.string.title_settings_appearance),
                            iconResId = R.drawable.icon_appearance,
                            onClick = {
                                // TODO:Navigate to appearance Settings screen
                            },
                        )
                        Spacer()
                        SettingsItem(
                            stringResource(R.string.title_settings_node),
                            iconResId = R.drawable.icon_node,
                            onClick = {
                                // TODO:Navigate to node Settings screen
                            },
                        )
                        Spacer()
                        SettingsItem(
                            stringResource(R.string.title_settings_currency),
                            iconResId = R.drawable.icon_currency,
                            onClick = {
                                // TODO:Navigate to currency Settings screen
                            },
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
