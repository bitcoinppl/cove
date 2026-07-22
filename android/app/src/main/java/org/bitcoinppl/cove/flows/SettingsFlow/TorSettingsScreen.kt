package org.bitcoinppl.cove.flows.SettingsFlow

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
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
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.CheckCircle
import androidx.compose.material.icons.filled.Error
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Button
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.RadioButton
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.res.stringResource
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.unit.dp
import org.bitcoinppl.cove.AppManager
import org.bitcoinppl.cove.R
import org.bitcoinppl.cove.TorManager
import org.bitcoinppl.cove.TorStatus
import org.bitcoinppl.cove.views.MaterialDivider
import org.bitcoinppl.cove.views.MaterialSection
import org.bitcoinppl.cove.views.MaterialSettingsItem
import org.bitcoinppl.cove.views.SectionHeader
import org.bitcoinppl.cove.views.ThemedSwitch
import org.bitcoinppl.cove_core.TorConfig
import org.bitcoinppl.cove_core.TorTestState
import org.bitcoinppl.cove_core.TorTestStep

private const val ORBOT_HOST = "127.0.0.1"
private const val ORBOT_PORT = 9050

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun TorSettingsScreen(
    app: AppManager,
    modifier: Modifier = Modifier,
    manager: TorManager = remember { TorManager.getInstance() },
) {
    val context = LocalContext.current
    val orbotPackage = remember(context) { OrbotPackageHelper.detect(context) }
    var externalHost by remember { mutableStateOf(ORBOT_HOST) }
    var externalPort by remember { mutableStateOf(ORBOT_PORT.toString()) }
    var externalConfigError by remember { mutableStateOf(false) }

    LaunchedEffect(manager.config) {
        val config = manager.config
        if (config is TorConfig.External) {
            externalHost = config.host
            externalPort = config.port.toString()
        }
    }

    fun applyExternalConfig(openOrbot: Boolean = false) {
        val port = externalPort.toIntOrNull()
        if (externalHost.isBlank() || port == null || port !in 1..UShort.MAX_VALUE.toInt()) {
            externalConfigError = true
            return
        }

        manager.applyConfig(TorConfig.External(externalHost.trim(), port.toUShort()))
        if (openOrbot && !OrbotPackageHelper.openOrbot(context)) {
            OrbotPackageHelper.openInstallPage(context)
        }
    }

    Scaffold(
        modifier =
            modifier
                .fillMaxSize()
                .padding(WindowInsets.safeDrawing.asPaddingValues()),
        topBar = {
            SettingsTopAppBar(
                title = stringResource(R.string.title_settings_tor),
                onBack = { app.popRoute() },
            )
        },
    ) { paddingValues ->
        Column(
            modifier =
                Modifier
                    .fillMaxSize()
                    .verticalScroll(rememberScrollState())
                    .padding(paddingValues),
        ) {
            SectionHeader(stringResource(R.string.tor_section_privacy), showDivider = false)
            MaterialSection {
                Column {
                    MaterialSettingsItem(
                        title = stringResource(R.string.tor_use_tor_title),
                        subtitle = stringResource(R.string.tor_use_tor_subtitle),
                        trailingContent = {
                            ThemedSwitch(
                                isChecked = manager.isEnabled,
                                onCheckChanged = { enabled ->
                                    if (enabled) manager.enable() else manager.disable()
                                },
                            )
                        },
                    )
                    MaterialDivider()
                    MaterialSettingsItem(
                        title = stringResource(R.string.tor_status_title),
                        subtitle = torStatusText(manager.config, manager.status),
                    )
                }
            }

            SectionHeader(stringResource(R.string.tor_mode_title))
            MaterialSection {
                Column {
                    TorModeRow(
                        title = stringResource(R.string.tor_mode_builtin),
                        selected = manager.config !is TorConfig.External,
                        onClick = { manager.applyConfig(TorConfig.BuiltIn) },
                    )
                    MaterialDivider()
                    TorModeRow(
                        title = stringResource(R.string.tor_mode_external),
                        selected = manager.config is TorConfig.External,
                        onClick = {
                            externalHost = externalHost.ifBlank { ORBOT_HOST }
                            externalPort = externalPort.ifBlank { ORBOT_PORT.toString() }
                            applyExternalConfig()
                        },
                    )
                }
            }

            if (manager.config is TorConfig.External) {
                SectionHeader(stringResource(R.string.tor_section_external))
                MaterialSection {
                    Column(
                        modifier = Modifier.padding(16.dp),
                        verticalArrangement = Arrangement.spacedBy(12.dp),
                    ) {
                        OutlinedTextField(
                            value = externalHost,
                            onValueChange = {
                                externalHost = it
                                externalConfigError = false
                            },
                            label = { Text(stringResource(R.string.tor_external_host_label)) },
                            singleLine = true,
                            modifier = Modifier.fillMaxWidth(),
                        )
                        OutlinedTextField(
                            value = externalPort,
                            onValueChange = {
                                externalPort = it
                                externalConfigError = false
                            },
                            label = { Text(stringResource(R.string.tor_external_port_label)) },
                            keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Number),
                            singleLine = true,
                            isError = externalConfigError,
                            supportingText =
                                if (externalConfigError) {
                                    { Text(stringResource(R.string.tor_error_config_invalid)) }
                                } else {
                                    null
                                },
                            modifier = Modifier.fillMaxWidth(),
                        )
                        Button(
                            onClick = { applyExternalConfig() },
                            modifier = Modifier.fillMaxWidth(),
                        ) {
                            Text(stringResource(R.string.tor_action_save_config))
                        }
                    }
                }

                SectionHeader(stringResource(R.string.tor_orbot_status_title))
                MaterialSection {
                    MaterialSettingsItem(
                        title = stringResource(R.string.tor_orbot_preset_title),
                        subtitle =
                            if (orbotPackage != null) {
                                orbotPackage.versionName?.let {
                                    stringResource(R.string.tor_orbot_detected_version, it)
                                } ?: stringResource(R.string.tor_orbot_detected)
                            } else {
                                stringResource(R.string.tor_orbot_not_detected)
                            },
                        leadingContent = {
                            Icon(
                                painter = painterResource(R.drawable.icon_tor_onion),
                                contentDescription = null,
                                tint = MaterialTheme.colorScheme.primary,
                            )
                        },
                        trailingContent = {
                            TextButton(
                                onClick = {
                                    if (orbotPackage != null) {
                                        externalHost = ORBOT_HOST
                                        externalPort = ORBOT_PORT.toString()
                                        applyExternalConfig(openOrbot = true)
                                    } else {
                                        OrbotPackageHelper.openInstallPage(context)
                                    }
                                },
                            ) {
                                Text(
                                    stringResource(
                                        if (orbotPackage != null) {
                                            R.string.tor_action_open_orbot
                                        } else {
                                            R.string.tor_action_install_orbot
                                        },
                                    ),
                                )
                            }
                        },
                    )
                }
            }

            val bootstrapping = manager.status as? TorStatus.Bootstrapping
            if (bootstrapping != null) {
                SectionHeader(stringResource(R.string.tor_section_bootstrap))
                MaterialSection {
                    Column(
                        modifier = Modifier.padding(16.dp),
                        verticalArrangement = Arrangement.spacedBy(8.dp),
                    ) {
                        LinearProgressIndicator(
                            progress = { bootstrapping.percent / 100f },
                            modifier = Modifier.fillMaxWidth(),
                        )
                        Text(
                            text = bootstrapping.message,
                            style = MaterialTheme.typography.bodySmall,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                    }
                }
            }

            if (manager.isEnabled) {
                SectionHeader(stringResource(R.string.tor_test_section))
                MaterialSection {
                    Column {
                        TorTestStepRow(
                            label = stringResource(R.string.tor_test_proxy),
                            state = manager.connectionTestStates[TorTestStep.PROXY_REACHABLE],
                        )
                        MaterialDivider()
                        TorTestStepRow(
                            label = stringResource(R.string.tor_test_node),
                            state = manager.connectionTestStates[TorTestStep.NODE_REACHABLE_VIA_TOR],
                        )
                        MaterialDivider()
                        Button(
                            onClick = manager::runConnectionTest,
                            enabled = !manager.isConnectionTestRunning,
                            modifier = Modifier.fillMaxWidth().padding(16.dp),
                        ) {
                            Text(stringResource(R.string.tor_action_test_connection))
                        }
                    }
                }
                Spacer(modifier = Modifier.height(24.dp))
            }
        }
    }

    if (manager.disableWarning != null) {
        AlertDialog(
            onDismissRequest = manager::dismissDisableWarning,
            title = { Text(stringResource(R.string.tor_disable_onion_dialog_title)) },
            text = { Text(stringResource(R.string.tor_disable_onion_dialog_body)) },
            confirmButton = {
                TextButton(onClick = manager::disableConfirmed) {
                    Text(stringResource(R.string.tor_disable_onion_dialog_confirm))
                }
            },
            dismissButton = {
                TextButton(onClick = manager::dismissDisableWarning) {
                    Text(stringResource(R.string.btn_cancel))
                }
            },
        )
    }

    manager.actionError?.let { error ->
        AlertDialog(
            onDismissRequest = manager::dismissActionError,
            title = { Text(stringResource(R.string.tor_error_title)) },
            text = { Text(error) },
            confirmButton = {
                TextButton(onClick = manager::dismissActionError) {
                    Text(stringResource(R.string.btn_ok))
                }
            },
        )
    }
}

@Composable
private fun TorModeRow(
    title: String,
    selected: Boolean,
    onClick: () -> Unit,
) {
    Row(
        modifier =
            Modifier
                .fillMaxWidth()
                .clickable(onClick = onClick)
                .padding(horizontal = 16.dp, vertical = 8.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        RadioButton(
            selected = selected,
            onClick = onClick,
        )
        Text(
            text = title,
            style = MaterialTheme.typography.bodyMedium,
            modifier = Modifier.padding(start = 8.dp),
        )
    }
}

@Composable
private fun TorTestStepRow(
    label: String,
    state: TorTestState?,
) {
    Row(
        modifier = Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 12.dp),
        horizontalArrangement = Arrangement.spacedBy(12.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        when (state) {
            null -> {
                Spacer(modifier = Modifier.size(20.dp))
            }

            is TorTestState.Running -> {
                CircularProgressIndicator(modifier = Modifier.size(20.dp), strokeWidth = 2.dp)
            }

            is TorTestState.Passed -> {
                Icon(
                    imageVector = Icons.Default.CheckCircle,
                    contentDescription = stringResource(R.string.tor_test_passed),
                    tint = MaterialTheme.colorScheme.primary,
                    modifier = Modifier.size(20.dp),
                )
            }

            is TorTestState.Failed -> {
                Icon(
                    imageVector = Icons.Default.Error,
                    contentDescription = stringResource(R.string.tor_test_failed),
                    tint = MaterialTheme.colorScheme.error,
                    modifier = Modifier.size(20.dp),
                )
            }
        }
        Column {
            Text(text = label, style = MaterialTheme.typography.bodyMedium)
            when (state) {
                is TorTestState.Running -> {
                    Text(
                        text = stringResource(R.string.tor_test_running),
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }

                is TorTestState.Passed -> {
                    Text(
                        text = stringResource(R.string.tor_test_passed),
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }

                is TorTestState.Failed -> {
                    Text(
                        text = state.v1,
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.error,
                    )
                }

                null -> {
                    Unit
                }
            }
        }
    }
}

@Composable
private fun torStatusText(
    config: TorConfig,
    status: TorStatus,
): String =
    when (status) {
        is TorStatus.Off -> {
            stringResource(R.string.tor_status_disabled)
        }

        is TorStatus.Bootstrapping -> {
            stringResource(R.string.tor_status_bootstrapping_detail, status.percent, status.message)
        }

        is TorStatus.Ready -> {
            stringResource(R.string.tor_status_ready)
        }

        is TorStatus.Stopped -> {
            if (config is TorConfig.External) {
                stringResource(R.string.tor_status_configured)
            } else {
                stringResource(R.string.tor_status_stopped)
            }
        }

        is TorStatus.Failed -> {
            stringResource(R.string.tor_status_error_detail, status.message)
        }
    }
