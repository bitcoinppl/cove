package org.bitcoinppl.cove.flows.SettingsFlow

import android.util.Log
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
import org.bitcoinppl.cove.ui.theme.MaterialSpacing
import org.bitcoinppl.cove.views.MaterialDivider
import org.bitcoinppl.cove.views.MaterialSection
import org.bitcoinppl.cove.views.SectionHeader
import org.bitcoinppl.cove_core.ApiType
import org.bitcoinppl.cove_core.Database
import org.bitcoinppl.cove_core.GlobalFlagKey
import org.bitcoinppl.cove_core.Node
import org.bitcoinppl.cove_core.NodeSelection
import org.bitcoinppl.cove_core.NodeSelector
import org.bitcoinppl.cove_core.NodeSelectorException
import org.bitcoinppl.cove_core.Route
import org.bitcoinppl.cove_core.SettingsRoute

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun NodeSettingsScreen(
    app: org.bitcoinppl.cove.AppManager,
    modifier: Modifier = Modifier,
) {
    val logTag = "NodeSettingsScreen"
    val nodeSelector = remember { NodeSelector() }
    val database = remember { Database() }
    val globalConfig = remember { database.globalConfig() }
    val globalFlag = remember { database.globalFlag() }
    val scope = rememberCoroutineScope()
    val snackbarHostState = remember { SnackbarHostState() }

    var nodeList by remember { mutableStateOf(nodeSelector.nodeList()) }
    var selectedNodeSelection by remember { mutableStateOf(nodeSelector.selectedNode()) }
    var selectedNodeName by remember {
        mutableStateOf(selectedNodeSelection.toNode().name)
    }

    var customUrl by remember { mutableStateOf("") }
    var customNodeName by remember { mutableStateOf("") }
    var suppressCustomDraftActions by remember { mutableStateOf(false) }

    var isLoading by remember { mutableStateOf(false) }
    var showErrorDialog by remember { mutableStateOf(false) }
    var errorMessage by remember { mutableStateOf("") }
    var errorTitle by remember { mutableStateOf("") }

    fun refreshNodeSelection(node: Node) {
        nodeList = NodeSelector().nodeList()
        selectedNodeSelection = NodeSelection.Custom(node)
        selectedNodeName = node.name
        suppressCustomDraftActions = true
        customUrl = ""
        customNodeName = ""
    }

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
    val torRedirectMessage = stringResource(R.string.node_tor_redirect_message)
    val nodeSavedViaTorNotice = stringResource(R.string.node_saved_via_tor_notice)

    val showCustomFields =
        selectedNodeSelection is NodeSelection.Custom ||
            selectedNodeName == customElectrum ||
            selectedNodeName == customEsplora

    // restore pending onion draft and auto-attempt save after Tor setup
    LaunchedEffect(Unit) {
        if (app.pendingNodeAwaitingTorSetup && app.pendingNodeUrl.isNotBlank()) {
            customUrl = app.pendingNodeUrl
            customNodeName = app.pendingNodeName
            selectedNodeName = app.pendingNodeTypeName
            Log.d(logTag, "restored pending onion draft: type=$selectedNodeName, ${redactedEndpointForLog(customUrl)}")

            if (globalConfig.useTor() && app.pendingNodeTorValidated) {
                isLoading = true
                try {
                    val pendingTypeName = app.pendingNodeTypeName.ifBlank { customElectrum }
                    val node =
                        withContext(Dispatchers.IO) {
                            nodeSelector.parseCustomNode(
                                app.pendingNodeUrl,
                                pendingTypeName,
                                app.pendingNodeName,
                            )
                        }

                    withContext(Dispatchers.IO) {
                        nodeSelector.checkAndSaveNode(node)
                    }

                    refreshNodeSelection(node)

                    app.pendingNodeAwaitingTorSetup = false
                    app.pendingNodeTorValidated = false
                    app.pendingNodeUrl = ""
                    app.pendingNodeName = ""
                    app.pendingNodeTypeName = ""

                    scope.launch {
                        snackbarHostState.showSnackbar(successSaved)
                    }
                    app.popRoute()
                } catch (e: NodeSelectorException.NodeAccessException) {
                    Log.e(logTag, "pending onion save failed: NodeAccess reason=${e.v1}", e)
                    errorTitle = errorConnectionFailed
                    errorMessage = errorConnectionMessage.format(e.v1)
                    showErrorDialog = true
                } catch (e: Exception) {
                    Log.e(logTag, "pending onion save failed: unexpected reason=${e.message}", e)
                    errorTitle = errorTitleDefault
                    errorMessage = errorUnknown.format(e.message ?: "")
                    showErrorDialog = true
                } finally {
                    isLoading = false
                }
            }
        }
    }

    // pre-fill custom fields if a custom node was previously saved
    LaunchedEffect(showCustomFields, selectedNodeSelection) {
        if (showCustomFields && customUrl.isEmpty() && !suppressCustomDraftActions) {
            val savedNode = selectedNodeSelection
            if (savedNode is NodeSelection.Custom) {
                val node = savedNode.toNode()
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
        Log.d(logTag, "selectPresetNode: nodeName=$nodeName")
        suppressCustomDraftActions = false
        selectedNodeName = nodeName
        customUrl = ""
        customNodeName = ""

        scope.launch {
            isLoading = true
            try {
                val node =
                    withContext(Dispatchers.IO) {
                        val selected = nodeSelector.selectPresetNode(nodeName)
                        Log.d(logTag, "selectPresetNode resolved: ${redactedNodeForLog(selected)}")
                        selected
                    }

                withContext(Dispatchers.IO) {
                    Log.d(logTag, "checkSelectedNode start: ${redactedNodeForLog(node)}")
                    nodeSelector.checkSelectedNode(node)
                    Log.d(logTag, "checkSelectedNode success: ${redactedNodeForLog(node)}")
                }
                selectedNodeSelection = NodeSelection.Preset(node)

                // launch snackbar in separate coroutine so it doesn't block finally
                scope.launch {
                    snackbarHostState.showSnackbar(
                        successConnected.format(node.url),
                    )
                }
            } catch (e: NodeSelectorException.NodeNotFound) {
                Log.e(logTag, "selectPresetNode failed: NodeNotFound name=$nodeName, reason=${e.v1}", e)
                errorTitle = errorTitleDefault
                errorMessage = errorNotFound.format(e.v1)
                showErrorDialog = true
            } catch (e: NodeSelectorException.NodeAccessException) {
                Log.e(logTag, "selectPresetNode failed: NodeAccess name=$nodeName, reason=${e.v1}", e)
                errorTitle = errorConnectionFailed
                errorMessage = errorConnectionMessage.format(e.v1)
                showErrorDialog = true
            } catch (e: Exception) {
                Log.e(logTag, "selectPresetNode failed: unexpected name=$nodeName, reason=${e.message}", e)
                errorTitle = errorTitleDefault
                errorMessage = errorUnknown.format(e.message ?: "")
                showErrorDialog = true
            } finally {
                isLoading = false
            }
        }
    }

    fun selectCustomNodeType(nodeName: String) {
        Log.d(logTag, "selectCustomNodeType: nodeName=$nodeName")
        suppressCustomDraftActions = false
        if (selectedNodeName != nodeName) {
            customUrl = ""
            customNodeName = ""
        }
        selectedNodeName = nodeName
    }

    fun checkAndSaveCustomNode() {
        Log.d(logTag, "checkAndSaveCustomNode: selectedNodeName=$selectedNodeName, ${redactedEndpointForLog(customUrl)}")
        if (customUrl.isEmpty()) {
            Log.e(logTag, "checkAndSaveCustomNode aborted: empty customUrl")
            errorTitle = errorTitleDefault
            errorMessage = errorUrlEmpty
            showErrorDialog = true
            return
        }

        scope.launch {
            isLoading = true
            try {
                val customNodeTypeName =
                    when {
                        selectedNodeName == customElectrum || selectedNodeName == customEsplora ->
                            selectedNodeName

                        selectedNodeSelection is NodeSelection.Custom ->
                            when (selectedNodeSelection.toNode().apiType) {
                                ApiType.ELECTRUM -> customElectrum
                                ApiType.ESPLORA -> customEsplora
                                else -> selectedNodeName
                            }

                        else -> selectedNodeName
                    }
                Log.d(logTag, "checkAndSaveCustomNode type inference: selectedNodeName=$selectedNodeName, customNodeTypeName=$customNodeTypeName, selectedApiType=${selectedNodeSelection.toNode().apiType}")

                val node =
                    withContext(Dispatchers.IO) {
                        Log.d(logTag, "parseCustomNode start: typeName=$customNodeTypeName, ${redactedEndpointForLog(customUrl)}")
                        val parsed = nodeSelector.parseCustomNode(customUrl, customNodeTypeName, customNodeName)
                        Log.d(logTag, "parseCustomNode success: ${redactedNodeForLog(parsed)}")
                        parsed
                    }

                // update fields with parsed values
                customUrl = node.url
                customNodeName = node.name

                val isOnionNode = isOnionNodeUrl(node.url)
                if (isOnionNode) {
                    if (globalConfig.useTor()) {
                        Log.d(logTag, "onion node detected with Tor already enabled; saving directly: ${redactedNodeForLog(node)}")
                        withContext(Dispatchers.IO) {
                            nodeSelector.checkAndSaveNode(node)
                        }
                        refreshNodeSelection(node)
                        app.pendingNodeAwaitingTorSetup = false
                        app.pendingNodeTorValidated = false
                        app.pendingNodeUrl = ""
                        app.pendingNodeName = ""
                        app.pendingNodeTypeName = ""
                        scope.launch {
                            snackbarHostState.showSnackbar(successSaved)
                        }
                        return@launch
                    }

                    Log.d(logTag, "onion node detected, redirecting to network settings: ${redactedNodeForLog(node)}")
                    runCatching {
                        globalFlag.set(GlobalFlagKey.TOR_SETTINGS_DISCOVERED, true)
                        globalConfig.setUseTor(true)
                    }.onFailure { error ->
                        Log.e(logTag, "failed to persist Tor setup before onion redirect: ${error.message}", error)
                        errorTitle = errorTitleDefault
                        errorMessage = errorUnknown.format(error.message ?: "")
                        showErrorDialog = true
                        return@launch
                    }

                    selectedNodeSelection = NodeSelection.Custom(node)
                    selectedNodeName =
                        when (node.apiType) {
                            ApiType.ELECTRUM -> customElectrum
                            ApiType.ESPLORA -> customEsplora
                            else -> node.name
                        }

                    app.pendingNodeUrl = node.url
                    app.pendingNodeName = node.name
                    app.pendingNodeTypeName = selectedNodeName
                    app.pendingNodeAwaitingTorSetup = true
                    app.pendingNodeTorValidated = false

                    scope.launch {
                        snackbarHostState.showSnackbar(torRedirectMessage)
                    }
                    app.pushRoute(Route.Settings(SettingsRoute.Network))
                    return@launch
                }

                withContext(Dispatchers.IO) {
                    Log.d(logTag, "checkAndSaveNode start: ${redactedNodeForLog(node)}")
                    nodeSelector.checkAndSaveNode(node)
                    Log.d(logTag, "checkAndSaveNode success: ${redactedNodeForLog(node)}")
                }
                refreshNodeSelection(node)
                Log.d(logTag, "custom node saved: selectedNodeName=$selectedNodeName, ${redactedNodeForLog(node)}")

                // launch snackbar in separate coroutine so it doesn't block finally
                scope.launch {
                    snackbarHostState.showSnackbar(successSaved)
                }
            } catch (e: NodeSelectorException.ParseNodeUrlException) {
                Log.e(logTag, "checkAndSaveCustomNode failed: ParseNodeUrl ${redactedEndpointForLog(customUrl)}, selectedNodeName=$selectedNodeName, reason=${e.v1}", e)
                errorTitle = errorParseTitle
                errorMessage = e.v1
                showErrorDialog = true
            } catch (e: NodeSelectorException.NodeAccessException) {
                Log.e(logTag, "checkAndSaveCustomNode failed: NodeAccess ${redactedEndpointForLog(customUrl)}, selectedNodeName=$selectedNodeName, reason=${e.v1}", e)
                errorTitle = errorConnectionFailed
                errorMessage = errorConnectionMessage.format(e.v1)
                showErrorDialog = true
            } catch (e: Exception) {
                Log.e(logTag, "checkAndSaveCustomNode failed: unexpected ${redactedEndpointForLog(customUrl)}, selectedNodeName=$selectedNodeName, reason=${e.message}", e)
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
                    Text(
                        style = MaterialTheme.typography.bodyLarge,
                        text = stringResource(R.string.title_settings_node),
                        textAlign = TextAlign.Center,
                        modifier = Modifier.fillMaxWidth(),
                    )
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
                        .padding(paddingValues),
            ) {
                SectionHeader(stringResource(R.string.title_settings_node), showDivider = false)
                MaterialSection {
                    Column {
                        // preset nodes
                        nodeList.forEachIndexed { index, nodeSelection ->
                            val node = nodeSelection.toNode()
                            NodeRow(
                                nodeName = node.name,
                                isSelected = selectedNodeName == node.name,
                                onClick = { selectPresetNode(node.name) },
                            )

                            if (index < nodeList.size - 1) {
                                MaterialDivider()
                            }
                        }

                        // add divider before custom options
                        if (nodeList.isNotEmpty()) {
                            MaterialDivider()
                        }

                        // custom electrum
                        NodeRow(
                            nodeName = customElectrum,
                            isSelected = selectedNodeName == customElectrum,
                            onClick = { selectCustomNodeType(customElectrum) },
                        )

                        MaterialDivider()

                        // custom esplora
                        NodeRow(
                            nodeName = customEsplora,
                            isSelected = selectedNodeName == customEsplora,
                            onClick = { selectCustomNodeType(customEsplora) },
                        )
                    }
                }

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
                                onValueChange = {
                                    suppressCustomDraftActions = false
                                    customUrl = it
                                },
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
                                onValueChange = {
                                    suppressCustomDraftActions = false
                                    customNodeName = it
                                },
                                label = { Text(stringResource(R.string.node_name_label)) },
                                placeholder = { Text(stringResource(R.string.node_name_placeholder)) },
                                keyboardOptions =
                                    KeyboardOptions(
                                        capitalization = KeyboardCapitalization.None,
                                    ),
                                singleLine = true,
                                modifier = Modifier.fillMaxWidth(),
                            )

                            if (suppressCustomDraftActions) {
                                Text(
                                    text = nodeSavedViaTorNotice,
                                    style = MaterialTheme.typography.bodySmall,
                                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                                )
                            } else {
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
