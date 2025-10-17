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
import uniffi.cove_core.ColorSchemeSelection

/**
 * appearance settings screen
 * allows user to select app theme (light, dark, system)
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun AppearanceSettingsScreen(
    app: AppManager,
    modifier: Modifier = Modifier
) {
    val currentTheme = app.colorSchemeSelection

    Scaffold(
        containerColor = Color.Transparent,
        topBar = {
            CenterAlignedTopAppBar(
                colors = TopAppBarDefaults.centerAlignedTopAppBarColors(
                    containerColor = Color.Transparent
                ),
                title = {
                    Text(
                        "Appearance",
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
                items = ColorSchemeSelection.entries,
                selectedItem = currentTheme,
                onItemSelected = { theme ->
                    app.dispatch(AppAction.ChangeColorScheme(theme))
                },
                itemLabel = { theme ->
                    when (theme) {
                        ColorSchemeSelection.LIGHT -> "Light"
                        ColorSchemeSelection.DARK -> "Dark"
                        ColorSchemeSelection.SYSTEM -> "System"
                    }
                },
                itemSymbol = { theme ->
                    when (theme) {
                        ColorSchemeSelection.LIGHT -> "☀️"
                        ColorSchemeSelection.DARK -> "🌙"
                        ColorSchemeSelection.SYSTEM -> "⚙️"
                    }
                }
            )
        }
    }
}
