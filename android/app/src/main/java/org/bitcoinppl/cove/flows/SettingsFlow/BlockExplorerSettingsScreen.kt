package org.bitcoinppl.cove.flows.SettingsFlow

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.asPaddingValues
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.safeDrawing
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.filled.Check
import androidx.compose.material3.Button
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Scaffold
import androidx.compose.material3.SnackbarHost
import androidx.compose.material3.SnackbarHostState
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalSoftwareKeyboardController
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.input.KeyboardCapitalization
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import org.bitcoinppl.cove.R
import org.bitcoinppl.cove.views.MaterialDivider
import org.bitcoinppl.cove.views.MaterialSection
import org.bitcoinppl.cove.views.SectionHeader
import org.bitcoinppl.cove_core.BlockExplorerOption
import org.bitcoinppl.cove_core.Database
import org.bitcoinppl.cove_core.allBlockExplorerOptions
import org.bitcoinppl.cove_core.types.Network

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun BlockExplorerSettingsScreen(
    app: org.bitcoinppl.cove.AppManager,
    modifier: Modifier = Modifier,
) {
    val config = remember { Database().globalConfig() }
    val scope = rememberCoroutineScope()
    val snackbarHostState = remember { SnackbarHostState() }
    val keyboardController = LocalSoftwareKeyboardController.current
    val savedMessage = stringResource(R.string.block_explorer_saved)

    // only Bitcoin explorer overrides are editable; other networks use built-in defaults
    val editableNetworks = remember { listOf(Network.BITCOIN) }
    var selectedNetwork by remember { mutableStateOf(Network.BITCOIN) }
    var input by remember(selectedNetwork) {
        mutableStateOf(config.customBlockExplorer(selectedNetwork) ?: "")
    }
    var preview by remember(selectedNetwork) {
        mutableStateOf(config.effectiveBlockExplorerPreview(selectedNetwork))
    }
    var selectedOption by remember(selectedNetwork) {
        mutableStateOf(config.selectedBlockExplorerOption(selectedNetwork))
    }
    var validationError by remember(selectedNetwork) { mutableStateOf<String?>(null) }
    var isSaving by remember(selectedNetwork) { mutableStateOf(false) }
    val blockExplorerOptions = remember { allBlockExplorerOptions() }

    fun reload() {
        input = config.customBlockExplorer(selectedNetwork) ?: ""
        preview = config.effectiveBlockExplorerPreview(selectedNetwork)
        selectedOption = config.selectedBlockExplorerOption(selectedNetwork)
        validationError = null
    }

    fun updatePreview(value: String) {
        try {
            preview = config.previewCustomBlockExplorer(selectedNetwork, value)
        } catch (_: Exception) {
            // keep save as the validation point while the user is still typing
        }

        validationError = null
    }

    fun save() {
        if (isSaving) return

        val inputToSave = input
        val networkToSave = selectedNetwork
        scope.launch {
            isSaving = true
            try {
                val normalized =
                    withContext(Dispatchers.IO) {
                        config.setCustomBlockExplorer(networkToSave, inputToSave)
                    }
                input = normalized ?: ""
                preview = config.effectiveBlockExplorerPreview(networkToSave)
                selectedOption = BlockExplorerOption.CUSTOM
                if (normalized == null) {
                    selectedOption = config.selectedBlockExplorerOption(networkToSave)
                }
                validationError = null
                keyboardController?.hide()
                isSaving = false

                launch {
                    snackbarHostState.showSnackbar(savedMessage)
                }
            } catch (error: Exception) {
                validationError = error.message ?: error.toString()
                isSaving = false
            }
        }
    }

    fun savePreset(option: BlockExplorerOption) {
        try {
            input = config.setBlockExplorerOption(selectedNetwork, option) ?: ""
            preview = config.effectiveBlockExplorerPreview(selectedNetwork)
            selectedOption = config.selectedBlockExplorerOption(selectedNetwork)
            validationError = null
        } catch (error: Exception) {
            validationError = error.message ?: error.toString()
        }
    }

    fun reset() {
        try {
            config.clearCustomBlockExplorer(selectedNetwork)
            reload()
        } catch (error: Exception) {
            validationError = error.message ?: error.toString()
        }
    }

    fun selectOption(option: BlockExplorerOption) {
        when (option) {
            BlockExplorerOption.CUSTOM -> {
                selectedOption = BlockExplorerOption.CUSTOM
                updatePreview(input)
            }
            else -> savePreset(option)
        }
    }

    Scaffold(
        modifier =
            modifier
                .fillMaxSize()
                .padding(WindowInsets.safeDrawing.asPaddingValues()),
        snackbarHost = { SnackbarHost(snackbarHostState) },
        topBar = {
            TopAppBar(
                title = {
                    Text(
                        style = MaterialTheme.typography.bodyLarge,
                        text = stringResource(R.string.title_settings_block_explorer),
                        textAlign = TextAlign.Center,
                        modifier = Modifier.fillMaxWidth(),
                    )
                },
                navigationIcon = {
                    IconButton(onClick = { app.popRoute() }) {
                        Icon(Icons.AutoMirrored.Default.ArrowBack, contentDescription = "Back")
                    }
                },
                actions = {
                    if (isSaving) {
                        Box(
                            modifier = Modifier.padding(end = 16.dp),
                            contentAlignment = Alignment.Center,
                        ) {
                            CircularProgressIndicator(
                                modifier = Modifier.width(24.dp).height(24.dp),
                            )
                        }
                    }
                },
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
            if (editableNetworks.size > 1) {
                SectionHeader(stringResource(R.string.title_settings_network), showDivider = false)
                MaterialSection {
                    Column {
                        editableNetworks.forEachIndexed { index, network ->
                            NetworkPickerRow(
                                network = network,
                                isSelected = selectedNetwork == network,
                                onClick = {
                                    selectedNetwork = network
                                },
                            )

                            if (index < editableNetworks.size - 1) {
                                MaterialDivider()
                            }
                        }
                    }
                }
            }

            SectionHeader(stringResource(R.string.block_explorer_description_title), showDivider = false)
            MaterialSection {
                Text(
                    text = stringResource(R.string.block_explorer_description),
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    modifier = Modifier.padding(horizontal = 16.dp, vertical = 12.dp),
                )
            }

            SectionHeader(stringResource(R.string.block_explorer_preview), showDivider = false)
            MaterialSection {
                Text(
                    text = preview,
                    style = MaterialTheme.typography.bodySmall,
                    fontFamily = FontFamily.Monospace,
                    modifier = Modifier.padding(horizontal = 16.dp, vertical = 12.dp),
                )
            }

            SectionHeader(stringResource(R.string.block_explorer_options))
            MaterialSection {
                Column {
                    blockExplorerOptions.forEachIndexed { index, option ->
                        BlockExplorerOptionRow(
                            option = option,
                            isSelected = selectedOption == option,
                            onClick = { selectOption(option) },
                        )

                        if (index < blockExplorerOptions.size - 1) {
                            MaterialDivider()
                        }
                    }
                }
            }

            if (selectedOption == BlockExplorerOption.CUSTOM) {
                SectionHeader(BlockExplorerOption.CUSTOM.displayName())
                MaterialSection {
                    Column(modifier = Modifier.padding(horizontal = 16.dp, vertical = 12.dp)) {
                        OutlinedTextField(
                            value = input,
                            onValueChange = {
                                input = it
                                selectedOption = BlockExplorerOption.CUSTOM
                                updatePreview(it)
                            },
                            label = { Text(stringResource(R.string.block_explorer_url_placeholder)) },
                            keyboardOptions =
                                KeyboardOptions(
                                    capitalization = KeyboardCapitalization.None,
                                    keyboardType = KeyboardType.Uri,
                                ),
                            minLines = 1,
                            maxLines = 4,
                            isError = validationError != null,
                            supportingText = validationError?.let { error -> { Text(error) } },
                            modifier = Modifier.fillMaxWidth(),
                        )

                        Row(
                            modifier =
                                Modifier
                                    .fillMaxWidth()
                                    .padding(top = 12.dp),
                            verticalAlignment = Alignment.CenterVertically,
                        ) {
                            Button(
                                onClick = ::save,
                                enabled = !isSaving,
                            ) {
                                Text(stringResource(R.string.block_explorer_save))
                            }

                            TextButton(onClick = ::reset) {
                                Text(stringResource(R.string.block_explorer_reset))
                            }
                        }
                    }
                }
            }
        }
    }
}

@Composable
private fun BlockExplorerOptionRow(
    option: BlockExplorerOption,
    isSelected: Boolean,
    onClick: () -> Unit,
) {
    Row(
        modifier =
            Modifier
                .fillMaxWidth()
                .clickable(onClick = onClick)
                .padding(horizontal = 16.dp, vertical = 12.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Text(
            text = option.displayName(),
            style = MaterialTheme.typography.bodyMedium,
            modifier = Modifier.weight(1f),
        )

        if (isSelected) {
            Icon(
                imageVector = Icons.Default.Check,
                contentDescription = null,
                tint = MaterialTheme.colorScheme.primary,
            )
        }
    }
}

@Composable
private fun NetworkPickerRow(
    network: Network,
    isSelected: Boolean,
    onClick: () -> Unit,
) {
    Row(
        modifier =
            Modifier
                .fillMaxWidth()
                .clickable(onClick = onClick)
                .padding(horizontal = 16.dp, vertical = 12.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Text(
            text = network.displayName(),
            style = MaterialTheme.typography.bodyLarge,
            modifier = Modifier.weight(1f),
        )

        if (isSelected) {
            Icon(
                imageVector = Icons.Default.Check,
                contentDescription = null,
                tint = MaterialTheme.colorScheme.primary,
            )
        }
    }
}
