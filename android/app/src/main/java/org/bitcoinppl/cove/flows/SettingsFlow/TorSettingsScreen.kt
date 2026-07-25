package org.bitcoinppl.cove.flows.SettingsFlow

import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.asPaddingValues
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.safeDrawing
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.Saver
import androidx.compose.runtime.saveable.listSaver
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.pluralStringResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import org.bitcoinppl.cove.AppManager
import org.bitcoinppl.cove.R
import org.bitcoinppl.cove.TorAlert
import org.bitcoinppl.cove.TorManager
import org.bitcoinppl.cove.TorStatus
import org.bitcoinppl.cove_core.TorConfig
import org.bitcoinppl.cove_core.TorDisableWarning

private const val ORBOT_HOST = "127.0.0.1"
private const val ORBOT_PORT = 9050

/**
 * Connection mode the user is editing, which stays ahead of the persisted config until an
 * external proxy is saved
 */
internal enum class TorMode {
    BUILT_IN,
    EXTERNAL,
    ;

    companion object {
        fun of(config: TorConfig): TorMode = if (config is TorConfig.External) EXTERNAL else BUILT_IN
    }
}

/**
 * External SOCKS5 proxy being edited, which only reaches the Tor config once it is saved
 */
internal data class ExternalProxyDraft(
    val host: String = ORBOT_HOST,
    val port: String = ORBOT_PORT.toString(),
    val hasError: Boolean = false,
) {
    fun toConfig(): TorConfig.External? {
        val host = host.trim()
        val port = port.toIntOrNull()

        if (host.isBlank() || port == null || port !in 1..UShort.MAX_VALUE.toInt()) return null

        return TorConfig.External(host, port.toUShort())
    }

    companion object {
        /** Keeps a half-typed proxy across a rotation, which recreates the activity */
        val Saver: Saver<ExternalProxyDraft, Any> =
            listSaver(
                save = { listOf(it.host, it.port, it.hasError) },
                restore = {
                    ExternalProxyDraft(
                        host = it[0] as String,
                        port = it[1] as String,
                        hasError = it[2] as Boolean,
                    )
                },
            )
    }
}

/**
 * Identifies a config across an activity recreation, which a `TorConfig` instance cannot do
 */
private fun TorConfig.seedKey(): String =
    when (this) {
        is TorConfig.Off -> "off"
        is TorConfig.BuiltIn -> "built-in"
        is TorConfig.External -> "external:$host:$port"
    }

@Composable
fun TorSettingsScreen(
    app: AppManager,
    modifier: Modifier = Modifier,
) {
    val manager = remember { TorManager.getInstanceOrNull() }

    if (manager == null) {
        TorUnavailableScreen(app = app, modifier = modifier)
        return
    }

    TorSettingsContent(app = app, manager = manager, modifier = modifier)
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun TorSettingsContent(
    app: AppManager,
    manager: TorManager,
    modifier: Modifier = Modifier,
) {
    val context = LocalContext.current
    val orbotPackage by rememberOrbotPackage()
    var proxyDraft by
        rememberSaveable(stateSaver = ExternalProxyDraft.Saver) {
            mutableStateOf(ExternalProxyDraft())
        }
    var selectedMode by rememberSaveable { mutableStateOf(TorMode.of(manager.config)) }
    var orbotInstallUnavailable by rememberSaveable { mutableStateOf(false) }

    // the effect below runs again on the composition a rotation creates, so the config it last
    // seeded from is kept to tell a real config change from a recreated activity
    var seededConfig by rememberSaveable { mutableStateOf<String?>(null) }

    LaunchedEffect(manager.config) {
        val config = manager.config
        val configKey = config.seedKey()
        if (configKey == seededConfig) return@LaunchedEffect

        seededConfig = configKey

        proxyDraft =
            if (config is TorConfig.External) {
                ExternalProxyDraft(host = config.host, port = config.port.toString())
            } else {
                proxyDraft.copy(hasError = false)
            }

        selectedMode = TorMode.of(config)
    }

    fun applyProxyDraft(openOrbot: Boolean = false) {
        val config = proxyDraft.toConfig()
        if (config == null) {
            proxyDraft = proxyDraft.copy(hasError = true)
            return
        }

        manager.applyConfig(config)

        if (openOrbot && !OrbotPackageHelper.openOrbot(context)) {
            orbotInstallUnavailable = !OrbotPackageHelper.openInstallPage(context)
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
            TorPrivacySection(manager)

            if (manager.autoStartSuppressed) {
                TorAutoStartSuppressedSection(
                    isStarting = manager.isUpdatingConfig,
                    onStartAgain = manager::enable,
                )
            }

            // the mode only means something while Tor is on, and showing it enabled-looking
            // while Tor is off makes a tap on it silently turn Tor on
            if (manager.isEnabled) {
                TorModeSection(
                    selectedMode = selectedMode,
                    onSelectBuiltIn = {
                        selectedMode = TorMode.BUILT_IN
                        proxyDraft = proxyDraft.copy(hasError = false)

                        if (manager.config !is TorConfig.BuiltIn) {
                            manager.applyConfig(TorConfig.BuiltIn)
                        }
                    },
                    // switching to external only reveals the proxy fields, the config is applied
                    // on save so an unedited 127.0.0.1:9050 is never persisted for a user who
                    // meant to type their own proxy
                    onSelectExternal = { selectedMode = TorMode.EXTERNAL },
                )
            }

            if (manager.isEnabled && selectedMode == TorMode.EXTERNAL) {
                TorExternalProxySection(
                    draft = proxyDraft,
                    onHostChange = { proxyDraft = ExternalProxyDraft(it, proxyDraft.port) },
                    onPortChange = { proxyDraft = ExternalProxyDraft(proxyDraft.host, it) },
                    onSave = { applyProxyDraft() },
                )

                TorOrbotSection(
                    orbotPackage = orbotPackage,
                    onUseOrbot = {
                        proxyDraft = ExternalProxyDraft()
                        applyProxyDraft(openOrbot = true)
                    },
                    onInstallOrbot = {
                        orbotInstallUnavailable = !OrbotPackageHelper.openInstallPage(context)
                    },
                )
            }

            (manager.status as? TorStatus.Bootstrapping)?.let { TorBootstrapSection(it) }

            if (manager.isEnabled) {
                TorConnectionTestSection(manager)
            }
        }
    }

    TorSettingsDialogs(
        manager = manager,
        isOrbotInstallUnavailable = orbotInstallUnavailable,
        onDismissOrbotInstallUnavailable = { orbotInstallUnavailable = false },
    )
}

/**
 * Shown when the Tor manager could not be created, so there is no state to configure
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun TorUnavailableScreen(
    app: AppManager,
    modifier: Modifier = Modifier,
) {
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
        Text(
            text = stringResource(R.string.tor_error_manager_unavailable),
            style = MaterialTheme.typography.bodyMedium,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            modifier = Modifier.padding(paddingValues).padding(16.dp),
        )
    }
}

@Composable
private fun TorSettingsDialogs(
    manager: TorManager,
    isOrbotInstallUnavailable: Boolean,
    onDismissOrbotInstallUnavailable: () -> Unit,
) {
    manager.disableWarning?.let { warning ->
        val networks =
            when (warning) {
                is TorDisableWarning.OnionNodesSelected -> warning.networks
            }

        AlertDialog(
            onDismissRequest = { manager.dismiss(TorAlert.DISABLE_WARNING) },
            title = { Text(stringResource(R.string.tor_disable_onion_dialog_title)) },
            text = {
                Text(
                    pluralStringResource(
                        R.plurals.tor_disable_onion_dialog_body,
                        networks.size,
                        networks.joinToString { it.displayName() },
                    ),
                )
            },
            confirmButton = {
                TextButton(onClick = manager::disableConfirmed) {
                    Text(stringResource(R.string.tor_disable_onion_dialog_confirm))
                }
            },
            dismissButton = {
                TextButton(onClick = { manager.dismiss(TorAlert.DISABLE_WARNING) }) {
                    Text(stringResource(R.string.btn_cancel))
                }
            },
        )
    }

    manager.actionError?.let { error ->
        AlertDialog(
            onDismissRequest = { manager.dismiss(TorAlert.ACTION_ERROR) },
            title = { Text(stringResource(R.string.tor_error_title)) },
            text = { Text(error) },
            confirmButton = {
                TextButton(onClick = { manager.dismiss(TorAlert.ACTION_ERROR) }) {
                    Text(stringResource(R.string.btn_ok))
                }
            },
        )
    }

    if (isOrbotInstallUnavailable) {
        AlertDialog(
            onDismissRequest = onDismissOrbotInstallUnavailable,
            title = { Text(stringResource(R.string.tor_error_title)) },
            text = { Text(stringResource(R.string.tor_error_orbot_install_failed)) },
            confirmButton = {
                TextButton(onClick = onDismissOrbotInstallUnavailable) {
                    Text(stringResource(R.string.btn_ok))
                }
            },
        )
    }
}
