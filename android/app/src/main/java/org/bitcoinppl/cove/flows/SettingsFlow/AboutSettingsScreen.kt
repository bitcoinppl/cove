package org.bitcoinppl.cove.flows.SettingsFlow

import android.content.Context
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
import androidx.compose.material.icons.filled.Delete
import androidx.compose.material.icons.filled.Refresh
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.tooling.preview.Preview
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import org.bitcoinppl.cove.BuildConfig
import org.bitcoinppl.cove.cloudbackup.AndroidCloudStorageAccess
import org.bitcoinppl.cove.cloudbackup.clearCloudBackupDriveAccountBinding
import org.bitcoinppl.cove.ui.theme.CoveTheme
import org.bitcoinppl.cove.views.MaterialDivider
import org.bitcoinppl.cove.views.MaterialSection
import org.bitcoinppl.cove.views.MaterialSettingsItem
import org.bitcoinppl.cove.views.SectionHeader
import org.bitcoinppl.cove_core.Database
import org.bitcoinppl.cove_core.GlobalFlagKey
import org.bitcoinppl.cove_core.RustCloudBackupManager
import org.bitcoinppl.cove_core.device.CloudAccessPolicy

private data class WipeCloudResult(
    val succeeded: Boolean,
    val message: String,
)

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun AboutSettingsScreen(
    app: org.bitcoinppl.cove.AppManager,
    modifier: Modifier = Modifier,
) {
    val context = LocalContext.current
    val coroutineScope = rememberCoroutineScope()
    var buildTapCount by remember { mutableIntStateOf(0) }
    var showBetaDialog by remember { mutableStateOf(false) }
    var showBetaEnabledDialog by remember { mutableStateOf(false) }
    var showWipeCloudDialog by remember { mutableStateOf(false) }
    var wipeCloudResult by remember { mutableStateOf<WipeCloudResult?>(null) }
    var showResetLocalStateDialog by remember { mutableStateOf(false) }
    var resetLocalStateMessage by remember { mutableStateOf<String?>(null) }
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

    AboutSettingsContent(
        version = BuildConfig.VERSION_NAME,
        buildNumber = BuildConfig.VERSION_CODE.toString(),
        gitCommit = app.gitShortHash,
        gitBranch = app.gitBranch,
        isBetaEnabled = isBetaEnabled,
        onBack = { app.popRoute() },
        onBuildNumberClick = {
            buildTapCount++
            if (buildTapCount >= 5) {
                buildTapCount = 0
                showBetaDialog = true
            }
        },
        onFeedbackClick = {
            val intent = Intent(Intent.ACTION_SENDTO).apply {
                data = Uri.parse("mailto:feedback@covebitcoinwallet.com")
            }
            context.startActivity(intent)
        },
        onWipeCloudBackupClick = { showWipeCloudDialog = true },
        onResetLocalBackupStateClick = { showResetLocalStateDialog = true },
        modifier = modifier,
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

    if (showWipeCloudDialog) {
        AlertDialog(
            onDismissRequest = { showWipeCloudDialog = false },
            title = { Text("Wipe Cloud Backup?") },
            text = { Text("Deletes all Google Drive backup files and resets local backup state") },
            confirmButton = {
                TextButton(
                    onClick = {
                        showWipeCloudDialog = false
                        coroutineScope.launch {
                            wipeCloudResult = debugWipeCloudBackup(context)
                        }
                    },
                ) {
                    Text("Wipe")
                }
            },
            dismissButton = {
                TextButton(onClick = { showWipeCloudDialog = false }) {
                    Text("Cancel")
                }
            },
        )
    }

    wipeCloudResult?.let { result ->
        AlertDialog(
            onDismissRequest = { wipeCloudResult = null },
            title = { Text(if (result.succeeded) "Cloud Backup Wiped" else "Cloud Backup Wipe Failed") },
            text = { Text(result.message) },
            confirmButton = {
                TextButton(onClick = { wipeCloudResult = null }) {
                    Text("OK")
                }
            },
        )
    }

    if (showResetLocalStateDialog) {
        AlertDialog(
            onDismissRequest = { showResetLocalStateDialog = false },
            title = { Text("Reset Local Backup State?") },
            text = {
                Text("Clears local keychain and DB backup state but keeps Google Drive files intact. Use this to test the recovery flow.")
            },
            confirmButton = {
                TextButton(
                    onClick = {
                        RustCloudBackupManager().use {
                            it.debugResetCloudBackupState()
                        }
                        clearCloudBackupDriveAccountBinding(context)
                        showResetLocalStateDialog = false
                        resetLocalStateMessage =
                            "Local backup state and Google Drive account selection reset. Google Drive files are untouched."
                    },
                ) {
                    Text("Reset")
                }
            },
            dismissButton = {
                TextButton(onClick = { showResetLocalStateDialog = false }) {
                    Text("Cancel")
                }
            },
        )
    }

    resetLocalStateMessage?.let { message ->
        AlertDialog(
            onDismissRequest = { resetLocalStateMessage = null },
            title = { Text("Local State Reset") },
            text = { Text(message) },
            confirmButton = {
                TextButton(onClick = { resetLocalStateMessage = null }) {
                    Text("OK")
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

@OptIn(ExperimentalMaterial3Api::class)
@Composable
internal fun AboutSettingsContent(
    version: String,
    buildNumber: String,
    gitCommit: String,
    gitBranch: String,
    isBetaEnabled: Boolean,
    onBack: () -> Unit,
    onBuildNumberClick: () -> Unit,
    onFeedbackClick: () -> Unit,
    onWipeCloudBackupClick: () -> Unit,
    onResetLocalBackupStateClick: () -> Unit,
    modifier: Modifier = Modifier,
) {
    Scaffold(
        modifier =
            modifier
                .fillMaxSize()
                .padding(WindowInsets.safeDrawing.asPaddingValues()),
        topBar = {
            SettingsTopAppBar(
                title = "About",
                onBack = onBack,
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
                            value = version,
                        )
                        MaterialDivider()
                        AboutRow(
                            label = "Build Number",
                            value = buildNumber,
                            onClick = onBuildNumberClick,
                        )
                        MaterialDivider()
                        AboutRow(
                            label = "Git Commit",
                            value = gitCommit,
                        )
                        if (BuildConfig.DEBUG && gitBranch.isNotBlank()) {
                            MaterialDivider()
                            AboutRow(
                                label = "Git Branch",
                                value = gitBranch,
                            )
                        }
                    }
                }

                SectionHeader("Support")
                MaterialSection {
                    Column {
                        AboutRow(
                            label = "Feedback",
                            value = "feedback@covebitcoinwallet.com",
                            valueStyle = MaterialTheme.typography.bodySmall,
                            onClick = onFeedbackClick,
                        )
                    }
                }

                if (isBetaEnabled) {
                    SectionHeader("Debug")
                    MaterialSection {
                        Column {
                            DebugRow(
                                title = "Wipe Cloud Backup",
                                color = MaterialTheme.colorScheme.error,
                                icon = {
                                    Icon(
                                        Icons.Default.Delete,
                                        contentDescription = null,
                                        tint = MaterialTheme.colorScheme.error,
                                    )
                                },
                                onClick = onWipeCloudBackupClick,
                            )
                            MaterialDivider()
                            DebugRow(
                                title = "Reset Local Backup State",
                                icon = {
                                    Icon(
                                        Icons.Default.Refresh,
                                        contentDescription = null,
                                        tint = MaterialTheme.colorScheme.primary,
                                    )
                                },
                                onClick = onResetLocalBackupStateClick,
                            )
                        }
                    }
                }
            }
        },
    )
}

@Preview(
    name = "About Settings",
    showSystemUi = true,
    widthDp = 393,
    heightDp = 852,
)
@Composable
private fun AboutSettingsScreenPreview() {
    AboutSettingsPreviewContent()
}

@Composable
internal fun AboutSettingsPreviewContent() {
    CoveTheme(darkTheme = false, dynamicColor = false) {
        AboutSettingsContent(
            version = "1.3.0",
            buildNumber = "18",
            gitCommit = "abc1234",
            gitBranch = "fix-lock-unlock",
            isBetaEnabled = true,
            onBack = { },
            onBuildNumberClick = { },
            onFeedbackClick = { },
            onWipeCloudBackupClick = { },
            onResetLocalBackupStateClick = { },
        )
    }
}

@Composable
private fun DebugRow(
    title: String,
    onClick: () -> Unit,
    icon: @Composable () -> Unit,
    color: Color? = null,
) {
    MaterialSettingsItem(
        title = title,
        titleColor = color,
        leadingContent = icon,
        onClick = onClick,
    )
}

private suspend fun debugWipeCloudBackup(context: Context): WipeCloudResult {
    return try {
        val cloudStorage = AndroidCloudStorageAccess(context)
        val namespaces = cloudStorage.listNamespaces(CloudAccessPolicy.CONSENT_ALLOWED)

        for (namespace in namespaces) {
            cloudStorage.deleteNamespace(namespace, CloudAccessPolicy.CONSENT_ALLOWED)
        }

        RustCloudBackupManager().use {
            it.debugResetCloudBackupState()
        }

        WipeCloudResult(succeeded = true, message = "All cloud backup data deleted and local state reset")
    } catch (error: Exception) {
        WipeCloudResult(
            succeeded = false,
            message = "Google Drive wipe failed: ${error.message ?: error.javaClass.simpleName}",
        )
    }
}

@Composable
private fun AboutRow(
    label: String,
    value: String,
    onClick: (() -> Unit)? = null,
    valueStyle: androidx.compose.ui.text.TextStyle = MaterialTheme.typography.bodyLarge,
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
