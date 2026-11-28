package org.bitcoinppl.cove.flows.SettingsFlow

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
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
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Scaffold
import androidx.compose.material3.SnackbarHost
import androidx.compose.material3.SnackbarHostState
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.input.KeyboardCapitalization
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import org.bitcoinppl.cove.NodeDiscoveryManager
import org.bitcoinppl.cove.Log
import org.bitcoinppl.cove.R
import org.bitcoinppl.cove.ui.theme.MaterialSpacing
import org.bitcoinppl.cove.views.MaterialDivider
import org.bitcoinppl.cove.views.MaterialSection
import org.bitcoinppl.cove.views.SectionHeader
import org.bitcoinppl.cove.views.lockaware.AlertDialog
import org.bitcoinppl.cove_core.ApiType
import org.bitcoinppl.cove_core.DiscoveredNode
import org.bitcoinppl.cove_core.NodeDiscoveryException
import org.bitcoinppl.cove_core.NodeDiscoveryState
import org.bitcoinppl.cove_core.NodeSelection
import org.bitcoinppl.cove_core.NodeSelector
import org.bitcoinppl.cove_core.NodeSelectorException
import org.bitcoinppl.cove.displayMessage
import java.util.Locale

private const val TAG = "NodeSettings"

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun NodeSettingsScreen(
    app: org.bitcoinppl.cove.AppManager,
    modifier: Modifier = Modifier,
) {
    val context = LocalContext.current
    val discovery = remember(context) { NodeDiscoveryManager(context) }
    DisposableEffect(discovery) {
        onDispose { discovery.close() }
    }

    val nodeSelector = remember { NodeSelector() }
    val scope = rememberCoroutineScope()
    val snackbarHostState = remember { SnackbarHostState() }

    var presetNodes by remember { mutableStateOf(nodeSelector.presetNodes()) }
    var selectedNodeSelection by remember { mutableStateOf(nodeSelector.selectedNode()) }
    var selectedRow by remember { mutableStateOf(NodeRowSelection.of(selectedNodeSelection)) }

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

    val customElectrumRow = NodeRowSelection.Custom(ApiType.ELECTRUM)
    val customEsploraRow = NodeRowSelection.Custom(ApiType.ESPLORA)

    // the api the custom form saves as, which only a custom row decides
    val customApiType: ApiType? = (selectedRow as? NodeRowSelection.Custom)?.apiType
    val showCustomFields = customApiType != null

    // pull a fresh snapshot because NodeSelector mutates persisted node settings directly
    fun refreshNodeState() {
        NodeSelector().use { refreshedNodeSelector ->
            presetNodes = refreshedNodeSelector.presetNodes()
            selectedNodeSelection = refreshedNodeSelector.selectedNode()
        }
        selectedRow = NodeRowSelection.of(selectedNodeSelection)
    }

    // pre-fill custom fields if a custom node was previously saved
    LaunchedEffect(showCustomFields, selectedNodeSelection) {
        if (showCustomFields && customUrl.isEmpty()) {
            val savedNode = selectedNodeSelection
            if (savedNode is NodeSelection.Custom) {
                val node = savedNode.toNode()
                if (node.apiType == customApiType) {
                    customUrl = node.url
                    customNodeName = node.name
                }
            }
        }
    }

    // one selection is checked at a time, as the iOS loading popup enforces, so a slow check
    // finishing late can't replace a node picked after it. Each selection claims isLoading before
    // launching, so a second tap in the same frame is already turned away
    fun selectPresetNode(nodeName: String) {
        if (isLoading) return

        val row = NodeRowSelection.Preset(nodeName)
        if (selectedRow == row && NodeRowSelection.of(selectedNodeSelection) == row) {
            return
        }

        selectedRow = row
        customUrl = ""
        customNodeName = ""

        isLoading = true
        scope.launch {
            try {
                val node =
                    withContext(Dispatchers.IO) {
                        nodeSelector.selectPresetNode(nodeName)
                    }

                withContext(Dispatchers.IO) {
                    nodeSelector.checkSelectedNode(node)
                }
                refreshNodeState()

                // launch snackbar in separate coroutine so it doesn't block finally
                scope.launch {
                    snackbarHostState.showSnackbar(
                        successConnected.format(node.url),
                    )
                }
            } catch (e: NodeSelectorException.NodeNotFound) {
                errorTitle = errorTitleDefault
                errorMessage = errorNotFound.format(e.v1)
                showErrorDialog = true
            } catch (e: NodeSelectorException.NodeAccessException) {
                // checkSelectedNode returns the failure without logging it, unlike checkAndSaveNode
                Log.w(TAG, "Preset node check failed for $nodeName: ${e.v1}")
                errorTitle = errorConnectionFailed
                errorMessage = errorConnectionMessage.format(e.v1)
                showErrorDialog = true
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                Log.e(TAG, "Preset node selection failed for $nodeName", e)
                errorTitle = errorTitleDefault
                errorMessage = errorUnknown.format(Locale.ROOT, e.displayMessage(""))
                showErrorDialog = true
            } finally {
                isLoading = false
            }
        }
    }

    // a found server already proved it serves this network, and picking it runs the same check
    // and save as a custom node
    fun selectDiscoveredNode(found: DiscoveredNode) {
        if (isLoading) return

        isLoading = true
        scope.launch {
            try {
                withContext(Dispatchers.IO) {
                    nodeSelector.checkAndSaveNode(found.node)
                }

                refreshNodeState()

                // a picked server is saved as a custom node, which the custom form shows
                customUrl = found.node.url
                customNodeName = found.node.name

                scope.launch {
                    snackbarHostState.showSnackbar(successSaved)
                }
            } catch (e: NodeSelectorException.NodeAccessException) {
                errorTitle = errorConnectionFailed
                errorMessage = errorConnectionMessage.format(e.v1)
                showErrorDialog = true
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                Log.e(TAG, "Saving a discovered node failed", e)
                errorTitle = errorTitleDefault
                errorMessage = errorUnknown.format(Locale.ROOT, e.displayMessage(""))
                showErrorDialog = true
            } finally {
                isLoading = false
            }
        }
    }

    fun checkAndSaveCustomNode() {
        // nothing to save without a custom row, or while another selection is being checked
        val apiType = customApiType?.takeUnless { isLoading } ?: return

        if (customUrl.isEmpty()) {
            errorTitle = errorTitleDefault
            errorMessage = errorUrlEmpty
            showErrorDialog = true
            return
        }

        isLoading = true
        scope.launch {
            try {
                val node =
                    withContext(Dispatchers.IO) {
                        nodeSelector.parseCustomNode(customUrl, apiType, customNodeName)
                    }

                // update fields with parsed values
                customUrl = node.url
                customNodeName = node.name

                withContext(Dispatchers.IO) {
                    nodeSelector.checkAndSaveNode(node)
                }
                refreshNodeState()

                // launch snackbar in separate coroutine so it doesn't block finally
                scope.launch {
                    snackbarHostState.showSnackbar(successSaved)
                }
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
                errorMessage = errorUnknown.format(Locale.ROOT, e.displayMessage(""))
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
            SettingsTopAppBar(
                title = stringResource(R.string.title_settings_node),
                onBack = { app.popRoute() },
                modifier = Modifier.height(56.dp),
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
                SectionHeader(stringResource(R.string.title_settings_node), showDivider = false)
                MaterialSection {
                    Column {
                        // preset nodes
                        presetNodes.forEachIndexed { index, node ->
                            NodeRow(
                                nodeName = node.name,
                                isSelected = selectedRow == NodeRowSelection.Preset(node.name),
                                onClick = { selectPresetNode(node.name) },
                            )

                            if (index < presetNodes.size - 1) {
                                MaterialDivider()
                            }
                        }

                        // add divider before custom options
                        if (presetNodes.isNotEmpty()) {
                            MaterialDivider()
                        }

                        // custom electrum
                        NodeRow(
                            nodeName = customElectrum,
                            isSelected = selectedRow == customElectrumRow,
                            onClick = { selectedRow = customElectrumRow },
                        )

                        MaterialDivider()

                        // custom esplora
                        NodeRow(
                            nodeName = customEsplora,
                            isSelected = selectedRow == customEsploraRow,
                            onClick = { selectedRow = customEsploraRow },
                        )
                    }
                }

                Spacer(modifier = Modifier.height(MaterialSpacing.medium))
                SectionHeader(stringResource(R.string.node_lan_section))
                LanNodesSection(
                    search = discovery.state,
                    selectedUrl = (selectedNodeSelection as? NodeSelection.Custom)?.v1?.url,
                    onSearch = { discovery.start() },
                    onStop = { discovery.stop() },
                    onSelect = { selectDiscoveredNode(it) },
                )

                // custom node input fields
                if (showCustomFields) {
                    Spacer(modifier = Modifier.height(MaterialSpacing.medium))

                    SectionHeader("Custom node")
                    MaterialSection {
                        Column(
                            modifier =
                                Modifier
                                    .fillMaxWidth()
                                    .padding(MaterialSpacing.medium),
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
private fun LanNodesSection(
    search: NodeDiscoveryState,
    selectedUrl: String?,
    onSearch: () -> Unit,
    onStop: () -> Unit,
    onSelect: (DiscoveredNode) -> Unit,
) {
    val isSearching = search is NodeDiscoveryState.Searching
    val found =
        when (search) {
            is NodeDiscoveryState.Searching -> search.found
            is NodeDiscoveryState.Stopped -> search.found
            is NodeDiscoveryState.Finished -> search.found
            NodeDiscoveryState.Idle, is NodeDiscoveryState.Failed -> emptyList()
        }

    MaterialSection {
        Column {
            found.forEach { server ->
                DiscoveredNodeRow(
                    found = server,
                    isSelected = server.node.url == selectedUrl,
                    onClick = { onSelect(server) },
                )

                MaterialDivider()
            }

            lanSearchMessage(search)?.let {
                Row(
                    modifier = Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 12.dp),
                    verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.spacedBy(12.dp),
                ) {
                    if (isSearching) {
                        CircularProgressIndicator(modifier = Modifier.width(16.dp).height(16.dp), strokeWidth = 2.dp)
                    }

                    Text(
                        text = it,
                        style = MaterialTheme.typography.bodyMedium,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }

                MaterialDivider()
            }

            // a running search can be stopped, keeping the servers it found
            val searchLabel =
                when (search) {
                    NodeDiscoveryState.Idle -> R.string.node_lan_find
                    is NodeDiscoveryState.Searching -> R.string.node_lan_stop
                    else -> R.string.node_lan_search_again
                }

            TextButton(
                onClick = if (isSearching) onStop else onSearch,
                modifier = Modifier.fillMaxWidth().padding(horizontal = 4.dp),
            ) {
                Text(
                    text = stringResource(searchLabel),
                    modifier = Modifier.fillMaxWidth(),
                )
            }
        }
    }
}

/** What to tell the person about the search, or nothing when its servers say it all */
@Composable
private fun lanSearchMessage(search: NodeDiscoveryState): String? =
    when (search) {
        NodeDiscoveryState.Idle, is NodeDiscoveryState.Stopped -> null
        is NodeDiscoveryState.Searching -> stringResource(R.string.node_lan_searching)
        is NodeDiscoveryState.Finished ->
            if (search.found.isEmpty()) stringResource(R.string.node_lan_none_found) else null
        is NodeDiscoveryState.Failed ->
            when (search.error) {
                is NodeDiscoveryException.NoLocalNetwork -> stringResource(R.string.node_lan_no_network)
                is NodeDiscoveryException.LocalNetworkBlocked -> stringResource(R.string.node_lan_blocked)
            }
    }

@Composable
private fun DiscoveredNodeRow(
    found: DiscoveredNode,
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
        Column(modifier = Modifier.weight(1f)) {
            Text(
                text = found.host,
                style = MaterialTheme.typography.bodyMedium,
            )

            val detail =
                found.software?.let { stringResource(R.string.node_lan_detail, it, found.address) } ?: found.address

            Text(
                text = detail,
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }

        if (isSelected) {
            Icon(
                imageVector = Icons.Default.Check,
                contentDescription = stringResource(R.string.content_description_selected),
                tint = MaterialTheme.colorScheme.primary,
            )
        }
    }
}

/**
 * The row selected on the node screen
 *
 * Kept apart from the saved node's name, which the person chooses, so a custom node named like a
 * preset or like a custom row still selects its own row and re-saves with its own api
 */
private sealed interface NodeRowSelection {
    data class Preset(
        val name: String,
    ) : NodeRowSelection

    data class Custom(
        val apiType: ApiType,
    ) : NodeRowSelection

    companion object {
        // a saved custom node selects the custom row for its api, which shows it in the form
        fun of(selection: NodeSelection): NodeRowSelection =
            when (selection) {
                is NodeSelection.Preset -> Preset(selection.v1.name)
                is NodeSelection.Custom -> Custom(selection.v1.apiType)
            }
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
