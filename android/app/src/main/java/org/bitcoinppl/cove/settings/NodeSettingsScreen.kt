package org.bitcoinppl.cove.settings

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
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
import androidx.compose.material3.AlertDialog
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
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.input.KeyboardCapitalization
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import org.bitcoinppl.cove.R
import org.bitcoinppl.cove.views.CardItem
import org.bitcoinppl.cove.views.CustomSpacer
import org.bitcoinppl.cove_core.ApiType
import org.bitcoinppl.cove_core.NodeSelection
import org.bitcoinppl.cove_core.NodeSelector
import org.bitcoinppl.cove_core.NodeSelectorException
import org.bitcoinppl.cove_core.nodeSelectionToNode

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun NodeSettingsScreen(
    app: org.bitcoinppl.cove.AppManager,
    modifier: Modifier = Modifier,
) {
    val nodeSelector = remember { NodeSelector() }
    val scope = rememberCoroutineScope()
    val snackbarHostState = remember { SnackbarHostState() }

    val nodeList = remember { nodeSelector.nodeList() }
    var selectedNodeSelection by remember { mutableStateOf(nodeSelector.selectedNode()) }
    var selectedNodeName by remember {
        mutableStateOf(nodeSelectionToNode(selectedNodeSelection).name)
    }

    var customUrl by remember { mutableStateOf("") }
    var customNodeName by remember { mutableStateOf("") }

    var isLoading by remember { mutableStateOf(false) }
    var showErrorDialog by remember { mutableStateOf(false) }
    var errorMessage by remember { mutableStateOf("") }
    var errorTitle by remember { mutableStateOf("") }

    // compute all string resources at composable level
    val customElectrum = stringResource(R.string.node_custom_electrum)
    val customEsplora = stringResource(R.string.node_custom_esplora)
    val successConnected = stringResource(R.string.node_success_connected)
    val successSaved = stringResource(R.string.node_success_saved)
    val errorTitleDefault = stringResource(R.string.node_error_title)
    val errorNotFound = stringResource(R.string.node_error_not_found)
    val errorConnectionFailed = stringResource(R.string.node_error_connection_failed)
    val errorConnectionMessage = stringResource(R.string.node_error_connection_message)
    val errorUnknown = stringResource(R.string.node_error_unknown)
    val errorUrlEmpty = stringResource(R.string.node_error_url_empty)
    val errorParseTitle = stringResource(R.string.node_error_parse_title)

    val showCustomFields =
        selectedNodeSelection is NodeSelection.Custom ||
            selectedNodeName == customElectrum ||
            selectedNodeName == customEsplora

    // pre-fill custom fields if a custom node was previously saved
    LaunchedEffect(showCustomFields, selectedNodeSelection) {
        if (showCustomFields && customUrl.isEmpty()) {
            val savedNode = selectedNodeSelection
            if (savedNode is NodeSelection.Custom) {
                val node = nodeSelectionToNode(savedNode)
                val matchesType =
                    when (selectedNodeName) {
                        customElectrum -> node.apiType == ApiType.ELECTRUM
                        customEsplora -> node.apiType == ApiType.ESPLORA
                        else -> true
                    }
                if (matchesType) {
                    customUrl = node.url
                    customNodeName = node.name
                }
            }
        }
    }

    fun selectPresetNode(nodeName: String) {
        selectedNodeName = nodeName
        customUrl = ""
        customNodeName = ""

        scope.launch {
            isLoading = true
            try {
                val node =
                    withContext(Dispatchers.IO) {
                        nodeSelector.selectPresetNode(nodeName)
                    }

                withContext(Dispatchers.IO) {
                    nodeSelector.checkSelectedNode(node)
                }
                selectedNodeSelection = NodeSelection.Preset(node)

                snackbarHostState.showSnackbar(
                    successConnected.format(node.url),
                )
            } catch (e: NodeSelectorException.NodeNotFound) {
                errorTitle = errorTitleDefault
                errorMessage = errorNotFound.format(e.v1)
                showErrorDialog = true
            } catch (e: NodeSelectorException.NodeAccessException) {
                errorTitle = errorConnectionFailed
                errorMessage = errorConnectionMessage.format(e.v1)
                showErrorDialog = true
            } catch (e: Exception) {
                errorTitle = errorTitleDefault
                errorMessage = errorUnknown.format(e.message ?: "")
                showErrorDialog = true
            } finally {
                isLoading = false
            }
        }
    }

    fun checkAndSaveCustomNode() {
        if (customUrl.isEmpty()) {
            errorTitle = errorTitleDefault
            errorMessage = errorUrlEmpty
            showErrorDialog = true
            return
        }

        scope.launch {
            isLoading = true
            try {
                val node =
                    withContext(Dispatchers.IO) {
                        nodeSelector.parseCustomNode(customUrl, selectedNodeName, customNodeName)
                    }

                // update fields with parsed values
                customUrl = node.url
                customNodeName = node.name

                withContext(Dispatchers.IO) {
                    nodeSelector.checkAndSaveNode(node)
                }
                selectedNodeSelection = NodeSelection.Custom(node)
                selectedNodeName = node.name

                snackbarHostState.showSnackbar(successSaved)
            } catch (e: NodeSelectorException.ParseNodeUrlException) {
                errorTitle = errorParseTitle
                errorMessage = e.v1
                showErrorDialog = true
            } catch (e: NodeSelectorException.NodeAccessException) {
                errorTitle = errorConnectionFailed
                errorMessage = errorConnectionMessage.format(e.v1)
                showErrorDialog = true
            } catch (e: Exception) {
                errorTitle = errorTitleDefault
                errorMessage = errorUnknown.format(e.message ?: "")
                showErrorDialog = true
            } finally {
                isLoading = false
            }
        }
    }

    Scaffold(
        modifier =
            modifier
                .fillMaxSize()
                .padding(WindowInsets.safeDrawing.asPaddingValues()),
        snackbarHost = { SnackbarHost(snackbarHostState) },
        topBar = @Composable {
            TopAppBar(
                title = {
                    Box(
                        modifier = Modifier.fillMaxSize(),
                        contentAlignment = Alignment.Center,
                    ) {
                        Text(
                            style = MaterialTheme.typography.bodyLarge,
                            text = stringResource(R.string.title_settings_node),
                            textAlign = TextAlign.Center,
                        )
                    }
                },
                navigationIcon = {
                    IconButton(onClick = { app.popRoute() }) {
                        Icon(
                            Icons.AutoMirrored.Default.ArrowBack,
                            contentDescription = stringResource(R.string.content_description_back),
                        )
                    }
                },
                actions = {
                    if (isLoading) {
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
                CardItem(stringResource(R.string.title_settings_node)) {
                    Column(
                        modifier = Modifier.padding(vertical = 8.dp),
                    ) {
                        // preset nodes
                        nodeList.forEachIndexed { index, nodeSelection ->
                            val node = nodeSelectionToNode(nodeSelection)
                            NodeRow(
                                nodeName = node.name,
                                isSelected = selectedNodeName == node.name,
                                onClick = { selectPresetNode(node.name) },
                            )

                            if (index < nodeList.size - 1) {
                                CustomSpacer(paddingValues = PaddingValues(start = 16.dp))
                            }
                        }

                        // add divider before custom options
                        if (nodeList.isNotEmpty()) {
                            CustomSpacer(paddingValues = PaddingValues(start = 16.dp))
                        }

                        // custom electrum
                        NodeRow(
                            nodeName = customElectrum,
                            isSelected = selectedNodeName == customElectrum,
                            onClick = {
                                selectedNodeName = customElectrum
                            },
                        )

                        CustomSpacer(paddingValues = PaddingValues(start = 16.dp))

                        // custom esplora
                        NodeRow(
                            nodeName = customEsplora,
                            isSelected = selectedNodeName == customEsplora,
                            onClick = {
                                selectedNodeName = customEsplora
                            },
                        )
                    }
                }

                // custom node input fields
                if (showCustomFields) {
                    Spacer(modifier = Modifier.height(16.dp))

                    CardItem(selectedNodeName) {
                        Column(
                            modifier =
                                Modifier
                                    .fillMaxWidth()
                                    .padding(16.dp),
                            verticalArrangement = Arrangement.spacedBy(12.dp),
                        ) {
                            OutlinedTextField(
                                value = customUrl,
                                onValueChange = { customUrl = it },
                                label = { Text(stringResource(R.string.node_url_label)) },
                                placeholder = { Text(stringResource(R.string.node_url_placeholder)) },
                                keyboardOptions =
                                    KeyboardOptions(
                                        keyboardType = KeyboardType.Uri,
                                        capitalization = KeyboardCapitalization.None,
                                    ),
                                singleLine = true,
                                modifier = Modifier.fillMaxWidth(),
                            )

                            OutlinedTextField(
                                value = customNodeName,
                                onValueChange = { customNodeName = it },
                                label = { Text(stringResource(R.string.node_name_label)) },
                                placeholder = { Text(stringResource(R.string.node_name_placeholder)) },
                                keyboardOptions =
                                    KeyboardOptions(
                                        capitalization = KeyboardCapitalization.None,
                                    ),
                                singleLine = true,
                                modifier = Modifier.fillMaxWidth(),
                            )

                            Button(
                                onClick = { checkAndSaveCustomNode() },
                                enabled = customUrl.isNotEmpty() && !isLoading,
                                modifier = Modifier.fillMaxWidth(),
                            ) {
                                Text(stringResource(R.string.node_save_button))
                            }
                        }
                    }
                }
            }
        },
    )

    if (showErrorDialog) {
        AlertDialog(
            onDismissRequest = { showErrorDialog = false },
            title = { Text(errorTitle) },
            text = { Text(errorMessage) },
            confirmButton = {
                TextButton(onClick = { showErrorDialog = false }) {
                    Text(stringResource(R.string.btn_ok))
                }
            },
        )
    }
}

@Composable
private fun NodeRow(
    nodeName: String,
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
            text = nodeName,
            style = MaterialTheme.typography.bodyLarge,
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
