@file:Suppress("PackageNaming")

package org.bitcoinppl.cove.flows.SettingsFlow

import androidx.annotation.StringRes
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.asPaddingValues
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.safeDrawing
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Check
import androidx.compose.material3.Button
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.input.KeyboardCapitalization
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.Job
import kotlinx.coroutines.launch
import org.bitcoinppl.cove.AppManager
import org.bitcoinppl.cove.R
import org.bitcoinppl.cove.TorManager
import org.bitcoinppl.cove.displayMessage
import org.bitcoinppl.cove.ui.theme.MaterialSpacing
import org.bitcoinppl.cove.views.MaterialDivider
import org.bitcoinppl.cove.views.MaterialSection
import org.bitcoinppl.cove.views.SectionHeader
import org.bitcoinppl.cove.views.lockaware.AlertDialog
import org.bitcoinppl.cove_core.TorManagerAction
import org.bitcoinppl.cove_core.TorManagerException
import org.bitcoinppl.cove_core.TorMode
import org.bitcoinppl.cove_core.TorStatus

/**
 * The option the mode picker shows
 *
 * Kept apart from the saved [TorMode] because picking Tor proxy only opens the proxy form, and
 * nothing is saved until Use Proxy
 */
private enum class TorModeOption(
    val testTag: String,
    @param:StringRes val title: Int,
    @param:StringRes val footer: Int,
    // the mode picking this option saves, null for the proxy, which needs an address first
    val mode: TorMode?,
) {
    OFF("tor.mode.off", R.string.tor_mode_off, R.string.tor_mode_off_footer, TorMode.Off),
    BUILT_IN("tor.mode.builtIn", R.string.tor_mode_built_in, R.string.tor_mode_built_in_footer, TorMode.BuiltIn),
    PROXY("tor.mode.proxy", R.string.tor_mode_proxy, R.string.tor_mode_proxy_footer, null),
    ;

    companion object {
        fun of(mode: TorMode?): TorModeOption? =
            when (mode) {
                null -> null
                is TorMode.Off -> OFF
                is TorMode.BuiltIn -> BUILT_IN
                is TorMode.Proxy -> PROXY
            }
    }
}

/** What Check Connection last found, while the saved mode stays the one it checked */
private sealed interface TorCheckState {
    data object Idle : TorCheckState

    data object Running : TorCheckState

    data class Finished(
        val message: String,
    ) : TorCheckState
}

@Composable
internal fun torModeRowValue(mode: TorMode?): String? =
    when (mode) {
        null -> null
        is TorMode.Off -> stringResource(R.string.tor_row_off)
        is TorMode.BuiltIn -> stringResource(R.string.tor_row_built_in)
        is TorMode.Proxy -> stringResource(R.string.tor_row_proxy)
    }

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun TorSettingsScreen(
    app: AppManager,
    manager: TorManager,
    modifier: Modifier = Modifier,
) {
    val savedMode = manager.mode
    val rejection = manager.modeRejection
    var selectedOption by remember { mutableStateOf(TorModeOption.of(savedMode)) }

    // the picker follows the saved mode, so a refused change snaps it back to the mode still saved
    LaunchedEffect(savedMode, rejection) {
        selectedOption = TorModeOption.of(savedMode)
    }

    fun selectOption(option: TorModeOption) {
        selectedOption = option

        // the proxy option only opens the form; Use Proxy saves it
        val mode = option.mode ?: return

        // every pick goes to Rust, which skips one matching the mode it last applied; a check
        // against the saved mode here would drop a quick reversal while a change is still saving
        manager.dispatch(TorManagerAction.SelectMode(mode))
    }

    Scaffold(
        modifier =
            modifier
                .fillMaxSize()
                .padding(WindowInsets.safeDrawing.asPaddingValues()),
        topBar = @Composable {
            SettingsTopAppBar(
                title = stringResource(R.string.title_settings_tor),
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
                TorModeSection(selectedOption = selectedOption, onSelect = ::selectOption)

                if (selectedOption == TorModeOption.PROXY) {
                    TorProxySection(manager = manager, savedMode = savedMode)
                }

                TorStatusSection(manager = manager, savedMode = savedMode)

                SectionHeader(stringResource(R.string.tor_section_about))
                MaterialSection {
                    TorFooterText(stringResource(R.string.tor_about))
                }
            }
        },
    )

    rejection?.let { shown ->
        AlertDialog(
            onDismissRequest = { manager.consumeModeRejection(shown) },
            title = { Text(stringResource(R.string.tor_mode_rejected_title)) },
            text = { Text(shown.item.displayMessage()) },
            confirmButton = {
                TextButton(onClick = { manager.consumeModeRejection(shown) }) {
                    Text(stringResource(R.string.btn_ok))
                }
            },
        )
    }
}

private fun savedProxyAddress(mode: TorMode?): String? = (mode as? TorMode.Proxy)?.address

@Composable
private fun TorModeSection(
    selectedOption: TorModeOption?,
    onSelect: (TorModeOption) -> Unit,
) {
    SectionHeader(stringResource(R.string.tor_section_mode), showDivider = false)
    MaterialSection {
        Column {
            TorModeOption.entries.forEachIndexed { index, option ->
                TorModeRow(
                    title = stringResource(option.title),
                    isSelected = option == selectedOption,
                    onClick = { onSelect(option) },
                    modifier = Modifier.testTag(option.testTag),
                )

                if (index < TorModeOption.entries.size - 1) {
                    MaterialDivider()
                }
            }

            selectedOption?.let { TorFooterText(stringResource(it.footer)) }
        }
    }
}

@Composable
private fun TorModeRow(
    title: String,
    isSelected: Boolean,
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
) {
    Row(
        modifier =
            modifier
                .fillMaxWidth()
                .clickable(onClick = onClick)
                .padding(horizontal = 16.dp, vertical = 12.dp),
        horizontalArrangement = Arrangement.SpaceBetween,
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Text(
            text = title,
            style = MaterialTheme.typography.bodyMedium,
        )

        if (isSelected) {
            Icon(
                imageVector = Icons.Default.Check,
                contentDescription = stringResource(R.string.content_description_selected),
                tint = MaterialTheme.colorScheme.primary,
            )
        }
    }
}

@Composable
private fun TorProxySection(
    manager: TorManager,
    savedMode: TorMode?,
) {
    var address by remember { mutableStateOf(savedProxyAddress(savedMode) ?: manager.defaultProxyAddress()) }
    var error by remember { mutableStateOf<String?>(null) }

    // a saved proxy shows in the form as Rust normalized it
    LaunchedEffect(savedMode) {
        savedProxyAddress(savedMode)?.let { address = it }
    }

    fun useProxy() {
        val normalized =
            try {
                manager.validateProxyAddress(address)
            } catch (e: TorManagerException) {
                error = e.displayMessage()
                return
            }

        error = null
        address = normalized
        manager.dispatch(TorManagerAction.SelectMode(TorMode.Proxy(normalized)))
    }

    SectionHeader(stringResource(R.string.tor_section_proxy))
    MaterialSection {
        Column(
            modifier =
                Modifier
                    .fillMaxWidth()
                    .padding(MaterialSpacing.medium),
            verticalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            TorProxyAddressField(
                address = address,
                error = error,
                onAddressChange = {
                    address = it
                    error = null
                },
            )

            Button(
                onClick = ::useProxy,
                modifier = Modifier.fillMaxWidth().testTag("tor.proxy.use"),
            ) {
                Text(
                    text = stringResource(R.string.tor_use_proxy),
                    textAlign = TextAlign.Center,
                    modifier = Modifier.fillMaxWidth(),
                )
            }
        }
    }
}

@Composable
private fun TorProxyAddressField(
    address: String,
    error: String?,
    onAddressChange: (String) -> Unit,
) {
    OutlinedTextField(
        value = address,
        onValueChange = onAddressChange,
        label = { Text(stringResource(R.string.tor_proxy_address_label)) },
        isError = error != null,
        supportingText =
            error?.let {
                {
                    Text(
                        text = it,
                        color = MaterialTheme.colorScheme.error,
                        modifier = Modifier.testTag("tor.proxy.error"),
                    )
                }
            },
        // a host:port address, so no autocorrect or capitals
        keyboardOptions =
            KeyboardOptions(
                keyboardType = KeyboardType.Uri,
                capitalization = KeyboardCapitalization.None,
                autoCorrectEnabled = false,
            ),
        singleLine = true,
        modifier = Modifier.fillMaxWidth().testTag("tor.proxy.address"),
    )
}

@Composable
private fun TorStatusSection(
    manager: TorManager,
    savedMode: TorMode?,
) {
    val scope = rememberCoroutineScope()
    val isTorText = stringResource(R.string.tor_check_is_tor)
    val notTorText = stringResource(R.string.tor_check_not_tor)

    var check by remember { mutableStateOf<TorCheckState>(TorCheckState.Idle) }
    var checkJob by remember { mutableStateOf<Job?>(null) }

    // a check describes the mode it ran under, so a new saved mode drops the result and stops a
    // check still running for the old one
    LaunchedEffect(savedMode) {
        checkJob?.cancel()
        checkJob = null
        check = TorCheckState.Idle
    }

    fun checkConnection() {
        checkJob?.cancel()
        check = TorCheckState.Running
        checkJob = scope.launch {
            check =
                try {
                    val result = manager.checkConnection()
                    TorCheckState.Finished(if (result.isTor) isTorText else notTorText)
                } catch (e: TorManagerException) {
                    TorCheckState.Finished(e.displayMessage())
                }
        }
    }

    SectionHeader(stringResource(R.string.tor_section_status))
    MaterialSection {
        Column(
            modifier =
                Modifier
                    .fillMaxWidth()
                    .padding(MaterialSpacing.medium),
            verticalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            TorStatusRow(status = manager.status, onRetry = { manager.dispatch(TorManagerAction.Restart) })

            if (savedMode != null && savedMode !is TorMode.Off) {
                TorCheckConnection(check = check, onCheck = ::checkConnection)
            }
        }
    }
}

@Composable
private fun TorStatusRow(
    status: TorStatus,
    onRetry: () -> Unit,
) {
    val label =
        when (status) {
            is TorStatus.Off -> stringResource(R.string.tor_status_off)
            is TorStatus.Starting -> stringResource(R.string.tor_status_starting, status.percent.toInt())
            is TorStatus.Ready -> stringResource(R.string.tor_status_ready)
            is TorStatus.Failed -> stringResource(R.string.tor_status_failed)
            is TorStatus.UsingProxy -> stringResource(R.string.tor_status_using_proxy)
        }

    Row(
        horizontalArrangement = Arrangement.spacedBy(MaterialSpacing.small),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        if (status is TorStatus.Starting) {
            CircularProgressIndicator(modifier = Modifier.size(16.dp), strokeWidth = 2.dp)
        }

        Text(
            text = label,
            style = MaterialTheme.typography.bodyMedium,
            modifier = Modifier.testTag("tor.status"),
        )
    }

    if (status is TorStatus.Failed) {
        Text(
            text = status.message,
            style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )

        OutlinedButton(onClick = onRetry, modifier = Modifier.testTag("tor.status.retry")) {
            Text(stringResource(R.string.tor_try_again))
        }
    }
}

@Composable
private fun TorCheckConnection(
    check: TorCheckState,
    onCheck: () -> Unit,
) {
    val isRunning = check is TorCheckState.Running

    OutlinedButton(
        onClick = onCheck,
        enabled = !isRunning,
        modifier = Modifier.fillMaxWidth().testTag("tor.checkConnection"),
    ) {
        if (isRunning) {
            CircularProgressIndicator(modifier = Modifier.size(16.dp), strokeWidth = 2.dp)
            Spacer(modifier = Modifier.width(MaterialSpacing.small))
        }

        Text(stringResource(R.string.tor_check_connection))
    }

    if (check is TorCheckState.Finished) {
        Text(
            text = check.message,
            style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            modifier = Modifier.testTag("tor.checkConnection.result"),
        )
    }
}

@Composable
private fun TorFooterText(text: String) {
    Text(
        text = text,
        style = MaterialTheme.typography.bodySmall,
        color = MaterialTheme.colorScheme.onSurfaceVariant,
        modifier = Modifier.padding(horizontal = 16.dp, vertical = 12.dp),
    )
}
