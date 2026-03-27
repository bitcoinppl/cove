package org.bitcoinppl.cove.flows.SettingsFlow

import android.content.Intent
import android.net.Uri
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
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.delay
import org.bitcoinppl.cove.BuildConfig
import org.bitcoinppl.cove.views.MaterialDivider
import org.bitcoinppl.cove.views.MaterialSection
import org.bitcoinppl.cove.views.SectionHeader
import org.bitcoinppl.cove_core.Database
import org.bitcoinppl.cove_core.GlobalFlagKey

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun AboutSettingsScreen(
    app: org.bitcoinppl.cove.AppManager,
    modifier: Modifier = Modifier,
) {
    val context = LocalContext.current
    var buildTapCount by remember { mutableIntStateOf(0) }
    var showBetaDialog by remember { mutableStateOf(false) }
    var showBetaEnabledDialog by remember { mutableStateOf(false) }
    var isBetaEnabled by remember {
        mutableStateOf(
            Database().globalFlag().getBoolConfig(GlobalFlagKey.BETA_FEATURES_ENABLED)
        )
    }
    var betaError by remember { mutableStateOf<String?>(null) }

    LaunchedEffect(buildTapCount) {
        if (buildTapCount > 0) {
            delay(2000)
            buildTapCount = 0
        }
    }

    Scaffold(
        modifier =
            modifier
                .fillMaxSize()
                .padding(WindowInsets.safeDrawing.asPaddingValues()),
        topBar = @Composable {
            TopAppBar(
                title = {
                    Text(
                        style = MaterialTheme.typography.bodyLarge,
                        text = "About",
                        textAlign = TextAlign.Center,
                        modifier = Modifier.fillMaxWidth(),
                    )
                },
                navigationIcon = {
                    IconButton(onClick = { app.popRoute() }) {
                        Icon(Icons.AutoMirrored.Default.ArrowBack, contentDescription = "Back")
                    }
                },
                actions = { },
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
                SectionHeader("App Info", showDivider = false)
                MaterialSection {
                    Column {
                        AboutRow(
                            label = "Version",
                            value = BuildConfig.VERSION_NAME,
                        )
                        MaterialDivider()
                        AboutRow(
                            label = "Build Number",
                            value = BuildConfig.VERSION_CODE.toString(),
                            onClick = {
                                buildTapCount++
                                if (buildTapCount >= 5) {
                                    buildTapCount = 0
                                    showBetaDialog = true
                                }
                            },
                        )
                        MaterialDivider()
                        AboutRow(
                            label = "Git Commit",
                            value = app.rust.gitShortHash(),
                        )
                    }
                }

                SectionHeader("Support")
                MaterialSection {
                    Column {
                        AboutRow(
                            label = "Feedback",
                            value = "feedback@covebitcoinwallet.com",
                            valueStyle = MaterialTheme.typography.bodySmall,
                            onClick = {
                                val intent = Intent(Intent.ACTION_SENDTO).apply {
                                    data = Uri.parse("mailto:feedback@covebitcoinwallet.com")
                                }
                                context.startActivity(intent)
                            },
                        )
                    }
                }
            }
        },
    )

    if (showBetaDialog) {
        val currentlyEnabled = isBetaEnabled
        AlertDialog(
            onDismissRequest = { showBetaDialog = false },
            title = { Text(if (currentlyEnabled) "Disable Beta Features?" else "Enable Beta Features?") },
            text = { Text(if (currentlyEnabled) "This will hide experimental features" else "This will enable experimental features") },
            confirmButton = {
                TextButton(onClick = {
                    val newValue = !currentlyEnabled
                    try {
                        Database().globalFlag().set(GlobalFlagKey.BETA_FEATURES_ENABLED, newValue)
                        isBetaEnabled = newValue
                    } catch (e: Exception) {
                        betaError = "Failed to update beta features: ${e.message}"
                    }
                    showBetaDialog = false
                    if (newValue) showBetaEnabledDialog = true
                }) {
                    Text(if (currentlyEnabled) "Disable" else "Enable")
                }
            },
            dismissButton = {
                TextButton(onClick = { showBetaDialog = false }) {
                    Text("Cancel")
                }
            },
        )
    }

    betaError?.let { error ->
        AlertDialog(
            onDismissRequest = { betaError = null },
            title = { Text("Something went wrong!") },
            text = { Text(error) },
            confirmButton = {
                TextButton(onClick = { betaError = null }) {
                    Text("OK")
                }
            },
        )
    }

    if (showBetaEnabledDialog) {
        AlertDialog(
            onDismissRequest = {
                showBetaEnabledDialog = false
                app.popRoute()
            },
            title = { Text("Beta Features Enabled") },
            text = { Text("Beta features have been enabled") },
            confirmButton = {
                TextButton(onClick = {
                    showBetaEnabledDialog = false
                    app.popRoute()
                }) {
                    Text("OK")
                }
            },
        )
    }
}

@Composable
private fun AboutRow(
    label: String,
    value: String,
    onClick: (() -> Unit)? = null,
    valueStyle: androidx.compose.ui.text.TextStyle = MaterialTheme.typography.bodyMedium,
) {
    Row(
        modifier =
            Modifier
                .fillMaxWidth()
                .then(
                    if (onClick != null) Modifier.clickable(onClick = onClick) else Modifier,
                )
                .padding(horizontal = 16.dp, vertical = 14.dp),
        horizontalArrangement = Arrangement.SpaceBetween,
    ) {
        Text(
            text = label,
            style = MaterialTheme.typography.bodyLarge,
        )
        Text(
            text = value,
            style = valueStyle,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
    }
}
