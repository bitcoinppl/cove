package org.bitcoinppl.cove.flows.SettingsFlow

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.asPaddingValues
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.safeDrawing
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Check
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import org.bitcoinppl.cove.R
import org.bitcoinppl.cove.views.MaterialDivider
import org.bitcoinppl.cove.views.MaterialSection
import org.bitcoinppl.cove.views.SectionHeader
import org.bitcoinppl.cove_core.AppAction
import org.bitcoinppl.cove_core.types.ColorSchemeSelection
import org.bitcoinppl.cove_core.types.allColorSchemes

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun AppearanceSettingsScreen(
    app: org.bitcoinppl.cove.AppManager,
    modifier: Modifier = Modifier,
) {
    val colorSchemes = remember { allColorSchemes() }
    val selectedColorScheme = app.colorSchemeSelection

    Scaffold(
        modifier =
            modifier
                .fillMaxSize()
                .padding(WindowInsets.safeDrawing.asPaddingValues()),
        topBar = @Composable {
            SettingsTopAppBar(
                title = stringResource(R.string.title_settings_appearance),
                onBack = { app.popRoute() },
            )
        },
        content = { paddingValues ->
            Column(
                modifier =
                    Modifier
                        .fillMaxSize()
                        .verticalScroll(rememberScrollState())
                        .padding(paddingValues),
            ) {
                SectionHeader(stringResource(R.string.title_settings_appearance), showDivider = false)
                MaterialSection {
                    Column {
                        colorSchemes.forEachIndexed { index, colorScheme ->
                            ColorSchemeRow(
                                colorScheme = colorScheme,
                                isSelected = colorScheme == selectedColorScheme,
                                onClick = {
                                    app.dispatch(AppAction.ChangeColorScheme(colorScheme))
                                },
                            )

                            // add divider between items, but not after the last one
                            if (index < colorSchemes.size - 1) {
                                MaterialDivider()
                            }
                        }
                    }
                }
            }
        },
    )
}

@Composable
private fun ColorSchemeRow(
    colorScheme: ColorSchemeSelection,
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
            text = colorScheme.capitalizedString(),
            style = MaterialTheme.typography.bodyMedium,
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
