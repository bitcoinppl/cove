package org.bitcoinppl.cove.flows.SettingsFlow

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.CheckCircle
import androidx.compose.material.icons.filled.Error
import androidx.compose.material.icons.filled.Warning
import androidx.compose.material3.Button
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Icon
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.RadioButton
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.unit.dp
import org.bitcoinppl.cove.R
import org.bitcoinppl.cove.TorManager
import org.bitcoinppl.cove.TorStatus
import org.bitcoinppl.cove.views.MaterialDivider
import org.bitcoinppl.cove.views.MaterialSection
import org.bitcoinppl.cove.views.MaterialSettingsItem
import org.bitcoinppl.cove.views.SectionHeader
import org.bitcoinppl.cove.views.ThemedSwitch
import org.bitcoinppl.cove.views.torBootstrapMessage
import org.bitcoinppl.cove_core.TorConfig
import org.bitcoinppl.cove_core.TorTestState
import org.bitcoinppl.cove_core.TorTestStep


@Composable
internal fun TorPrivacySection(manager: TorManager) {
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
}

@Composable
internal fun TorAutoStartSuppressedSection(
    isStarting: Boolean,
    onStartAgain: () -> Unit,
) {
    SectionHeader(stringResource(R.string.tor_auto_start_section))
    MaterialSection {
        Column {
            MaterialSettingsItem(
                title = stringResource(R.string.tor_auto_start_suppressed_title),
                subtitle = stringResource(R.string.tor_auto_start_suppressed_body),
                leadingContent = {
                    Icon(
                        imageVector = Icons.Default.Warning,
                        contentDescription = null,
                        tint = MaterialTheme.colorScheme.error,
                    )
                },
            )
            MaterialDivider()
            Button(
                onClick = onStartAgain,
                enabled = !isStarting,
                modifier = Modifier.fillMaxWidth().padding(16.dp),
            ) {
                Text(
                    stringResource(
                        if (isStarting) {
                            R.string.tor_auto_start_suppressed_action_starting
                        } else {
                            R.string.tor_auto_start_suppressed_action
                        },
                    ),
                )
            }
        }
    }
}

@Composable
internal fun TorModeSection(
    selectedMode: TorMode,
    onSelectBuiltIn: () -> Unit,
    onSelectExternal: () -> Unit,
) {
    SectionHeader(stringResource(R.string.tor_mode_title))
    MaterialSection {
        Column {
            TorModeRow(
                title = stringResource(R.string.tor_mode_builtin),
                selected = selectedMode == TorMode.BUILT_IN,
                onClick = onSelectBuiltIn,
            )
            MaterialDivider()
            TorModeRow(
                title = stringResource(R.string.tor_mode_external),
                selected = selectedMode == TorMode.EXTERNAL,
                onClick = onSelectExternal,
            )
        }
    }
}

@Composable
internal fun TorExternalProxySection(
    draft: ExternalProxyDraft,
    onHostChange: (String) -> Unit,
    onPortChange: (String) -> Unit,
    onSave: () -> Unit,
) {
    SectionHeader(stringResource(R.string.tor_section_external))
    MaterialSection {
        Column(
            modifier = Modifier.padding(16.dp),
            verticalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            OutlinedTextField(
                value = draft.host,
                onValueChange = onHostChange,
                label = { Text(stringResource(R.string.tor_external_host_label)) },
                singleLine = true,
                modifier = Modifier.fillMaxWidth(),
            )
            OutlinedTextField(
                value = draft.port,
                onValueChange = onPortChange,
                label = { Text(stringResource(R.string.tor_external_port_label)) },
                keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Number),
                singleLine = true,
                isError = draft.hasError,
                supportingText =
                    if (draft.hasError) {
                        { Text(stringResource(R.string.tor_error_config_invalid)) }
                    } else {
                        null
                    },
                modifier = Modifier.fillMaxWidth(),
            )
            Button(
                onClick = onSave,
                modifier = Modifier.fillMaxWidth(),
            ) {
                Text(stringResource(R.string.tor_action_save_config))
            }
        }
    }
}

@Composable
internal fun TorOrbotSection(
    orbotPackage: OrbotPackage?,
    onUseOrbot: () -> Unit,
    onInstallOrbot: () -> Unit,
) {
    SectionHeader(stringResource(R.string.tor_orbot_status_title))
    MaterialSection {
        MaterialSettingsItem(
            title = stringResource(R.string.tor_orbot_preset_title),
            subtitle =
                when (orbotPackage) {
                    null -> stringResource(R.string.tor_orbot_not_detected)
                    else ->
                        orbotPackage.versionName?.let {
                            stringResource(R.string.tor_orbot_detected_version, it)
                        } ?: stringResource(R.string.tor_orbot_detected)
                },
            leadingContent = {
                Icon(
                    painter = painterResource(R.drawable.icon_tor_onion),
                    contentDescription = null,
                    tint = MaterialTheme.colorScheme.primary,
                )
            },
            trailingContent = {
                TextButton(onClick = if (orbotPackage != null) onUseOrbot else onInstallOrbot) {
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

@Composable
internal fun TorBootstrapSection(status: TorStatus.Bootstrapping) {
    SectionHeader(stringResource(R.string.tor_section_bootstrap))
    MaterialSection {
        Column(
            modifier = Modifier.padding(16.dp),
            verticalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            LinearProgressIndicator(
                progress = { status.percent / 100f },
                modifier = Modifier.fillMaxWidth(),
            )
            Text(
                text = torBootstrapMessage(status),
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
    }
}

@Composable
internal fun TorConnectionTestSection(manager: TorManager) {
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
            stringResource(
                R.string.tor_status_bootstrapping_detail,
                status.percent,
                torBootstrapMessage(status),
            )
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
