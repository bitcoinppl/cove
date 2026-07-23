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
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Button
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.LinearProgressIndicator
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
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.KeyboardCapitalization
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import org.bitcoinppl.cove.NodeScannerManager
import org.bitcoinppl.cove.R
import org.bitcoinppl.cove.ui.theme.MaterialSpacing
import org.bitcoinppl.cove.views.MaterialDivider
import org.bitcoinppl.cove.views.MaterialSection
import org.bitcoinppl.cove.views.SectionHeader
import org.bitcoinppl.cove_core.ApiType
import org.bitcoinppl.cove_core.CertificateDecision
import org.bitcoinppl.cove_core.FoundNode
import org.bitcoinppl.cove_core.FoundNodeTransport
import org.bitcoinppl.cove_core.InternalException
import org.bitcoinppl.cove_core.Node
import org.bitcoinppl.cove_core.NodeCertificate
import org.bitcoinppl.cove_core.NodeScanProgress
import org.bitcoinppl.cove_core.NodeScannerException
import org.bitcoinppl.cove_core.NodeScannerResult
import org.bitcoinppl.cove_core.NodeScannerState
import org.bitcoinppl.cove_core.NodeSelection
import org.bitcoinppl.cove_core.NodeSelector
import org.bitcoinppl.cove_core.NodeSelectorException
import org.bitcoinppl.cove_core.TlsTrust
import org.bitcoinppl.cove_core.TrustRequiredEndpoint
import java.util.Locale

private sealed interface PendingNodeCertificate {
    val certificate: NodeCertificate

    data class Custom(
        override val certificate: NodeCertificate,
    ) : PendingNodeCertificate

    data class Discovered(
        val candidate: TrustRequiredEndpoint,
    ) : PendingNodeCertificate {
        override val certificate: NodeCertificate
            get() = candidate.certificate
    }
}

private sealed interface DiscoveredNodeAttempt {
    data class Verified(
        val node: FoundNode,
    ) : DiscoveredNodeAttempt

    data class TrustRequired(
        val candidate: TrustRequiredEndpoint,
    ) : DiscoveredNodeAttempt
}

private data class NodeScanCallbacks(
    val toggleScan: () -> Unit,
    val selectVerified: (FoundNode) -> Unit,
    val selectTrustRequired: (TrustRequiredEndpoint) -> Unit,
)

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun NodeSettingsScreen(
    app: org.bitcoinppl.cove.AppManager,
    modifier: Modifier = Modifier,
) {
    val nodeSelector = remember { NodeSelector() }
    val nodeScanner = remember { NodeScannerManager() }
    val scope = rememberCoroutineScope()
    val snackbarHostState = remember { SnackbarHostState() }

    var nodeList by remember { mutableStateOf(nodeSelector.nodeList()) }
    var selectedNodeSelection by remember { mutableStateOf(nodeSelector.selectedNode()) }
    var selectedNodeName by remember {
        mutableStateOf(selectedNodeSelection.toNode().name)
    }

    var customUrl by remember { mutableStateOf("") }
    var customNodeName by remember { mutableStateOf("") }

    var isLoading by remember { mutableStateOf(false) }
    var showErrorDialog by remember { mutableStateOf(false) }
    var pendingCertificate by remember { mutableStateOf<PendingNodeCertificate?>(null) }
    var discoveredNodeAttempt by remember { mutableStateOf<DiscoveredNodeAttempt?>(null) }
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
    val certificateTitle = stringResource(R.string.node_certificate_title)
    val certificateMessage = stringResource(R.string.node_certificate_message)
    val certificateTrust = stringResource(R.string.node_certificate_trust)
    val certificateCancel = stringResource(R.string.node_certificate_cancel)
    val errorCertificateRead = stringResource(R.string.node_error_certificate_read)
    val certificateChangedTitle = stringResource(R.string.node_certificate_changed_title)
    val certificateChangedMessage = stringResource(R.string.node_certificate_changed_message)
    val scanErrorNoLocalNetwork = stringResource(R.string.node_scan_error_no_local_network)
    val scanErrorPermissionDenied = stringResource(R.string.node_scan_error_permission_denied)
    val scanErrorResultUnavailable = stringResource(R.string.node_scan_error_result_unavailable)
    val scanErrorNetworkChanged = stringResource(R.string.node_scan_error_network_changed)
    val scanErrorVerificationFailed = stringResource(R.string.node_scan_error_verification_failed)

    val showCustomFields =
        selectedNodeSelection is NodeSelection.Custom ||
            selectedNodeName == customElectrum ||
            selectedNodeName == customEsplora

    LaunchedEffect(nodeScanner) {
        // TODO(targetSdk 37): declare and runtime-request ACCESS_LOCAL_NETWORK,
        // handle rationale/denial/revocation, and gate scanning
        nodeScanner.start()
    }

    DisposableEffect(nodeScanner) {
        onDispose {
            nodeScanner.close()
        }
    }

    // pull a fresh snapshot because NodeSelector mutates persisted node settings directly
    fun refreshNodeState() {
        NodeSelector().use { refreshedNodeSelector ->
            nodeList = refreshedNodeSelector.nodeList()
            selectedNodeSelection = refreshedNodeSelector.selectedNode()
        }
        selectedNodeName = selectedNodeSelection.toNode().name
    }

    // pre-fill custom fields if a custom node was previously saved
    LaunchedEffect(showCustomFields, selectedNodeSelection) {
        if (showCustomFields && customUrl.isEmpty()) {
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
        if (selectedNodeSelection is NodeSelection.Preset && selectedNodeName == nodeName) {
            return
        }

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

    fun showCertificateReadError(message: String?) {
        errorTitle = errorConnectionFailed
        errorMessage = String.format(Locale.US, errorCertificateRead, message.orEmpty())
        showErrorDialog = true
    }

    // Whether the certificate can be offered for confirmation is decided in the
    // core, so both apps apply the same rule.
    suspend fun offerCertificate() {
        try {
            when (val decision = withContext(Dispatchers.IO) { nodeSelector.certificateDecision(customUrl) }) {
                is CertificateDecision.Unrecognized -> {
                    pendingCertificate = PendingNodeCertificate.Custom(decision.certificate)
                }

                is CertificateDecision.Changed -> {
                    errorTitle = certificateChangedTitle
                    errorMessage = certificateChangedMessage
                    showErrorDialog = true
                }
            }
        } catch (readError: NodeSelectorException) {
            showCertificateReadError(readError.message)
        } catch (readError: InternalException) {
            showCertificateReadError(readError.message)
        }
    }

    fun checkAndSaveCustomNode(acceptedTls: TlsTrust? = null) {
        if (customUrl.isEmpty()) {
            errorTitle = errorTitleDefault
            errorMessage = errorUrlEmpty
            showErrorDialog = true
            return
        }

        scope.launch {
            isLoading = true
            try {
                // the saved node's display name stays selected on re-entry, so the
                // localized label alone cannot tell an electrum node from esplora
                val savedCustomNode = (selectedNodeSelection as? NodeSelection.Custom)?.toNode()
                val savingElectrum =
                    selectedNodeName == customElectrum ||
                        (savedCustomNode?.apiType == ApiType.ELECTRUM && savedCustomNode.name == selectedNodeName)

                val node =
                    withContext(Dispatchers.IO) {
                        val tls =
                            acceptedTls
                                ?: if (savingElectrum) {
                                    nodeSelector.trustedCertificate(customUrl)
                                } else {
                                    null
                                }

                        nodeSelector.parseCustomNode(
                            customUrl,
                            selectedNodeName,
                            customNodeName,
                            tls,
                        )
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
            } catch (e: NodeSelectorException.CertificateNotTrusted) {
                android.util.Log.d("NodeSettings", "certificate not trusted, offering it", e)
                offerCertificate()
            } catch (e: Exception) {
                errorTitle = errorTitleDefault
                errorMessage = errorUnknown.format(e.message ?: "")
                showErrorDialog = true
            } finally {
                isLoading = false
            }
        }
    }

    fun nodeScannerErrorMessage(error: NodeScannerException): String =
        when (error) {
            is NodeScannerException.NoLocalNetwork -> scanErrorNoLocalNetwork
            is NodeScannerException.LocalNetworkPermissionDenied -> scanErrorPermissionDenied
            is NodeScannerException.ScanFailed ->
                String.format(Locale.US, scanErrorVerificationFailed, error.v1)
            is NodeScannerException.ResultUnavailable -> scanErrorResultUnavailable
            is NodeScannerException.NetworkChanged -> scanErrorNetworkChanged
        }

    fun connectToDiscoveredNode(
        attempt: DiscoveredNodeAttempt,
        createNode: suspend () -> Node,
    ) {
        if (discoveredNodeAttempt != null || isLoading) return

        discoveredNodeAttempt = attempt
        scope.launch {
            isLoading = true

            try {
                val node = createNode()

                withContext(Dispatchers.IO) {
                    nodeSelector.checkAndSaveNode(node)
                }
                refreshNodeState()

                (selectedNodeSelection as? NodeSelection.Custom)?.toNode()?.let { selectedNode ->
                    customUrl = selectedNode.url
                    customNodeName = selectedNode.name
                }

                scope.launch {
                    snackbarHostState.showSnackbar(String.format(Locale.US, successConnected, node.url))
                }
            } catch (error: NodeScannerException) {
                errorTitle = errorConnectionFailed
                errorMessage = String.format(Locale.US, errorConnectionMessage, nodeScannerErrorMessage(error))
                showErrorDialog = true
            } catch (error: NodeSelectorException.NodeAccessException) {
                errorTitle = errorConnectionFailed
                errorMessage = String.format(Locale.US, errorConnectionMessage, error.v1)
                showErrorDialog = true
            } catch (e: Exception) {
                errorTitle = errorTitleDefault
                errorMessage = String.format(Locale.US, errorUnknown, e.message.orEmpty())
                showErrorDialog = true
            } finally {
                discoveredNodeAttempt = null
                isLoading = false
            }
        }
    }

    fun connectToDiscoveredNode(found: FoundNode) {
        connectToDiscoveredNode(DiscoveredNodeAttempt.Verified(found)) {
            nodeScanner.createNode(found)
        }
    }

    fun connectToDiscoveredNode(candidate: TrustRequiredEndpoint) {
        connectToDiscoveredNode(DiscoveredNodeAttempt.TrustRequired(candidate)) {
            nodeScanner.trustAndCreateNode(candidate)
        }
    }

    fun presentDiscoveredCertificate(candidate: TrustRequiredEndpoint) {
        if (discoveredNodeAttempt != null || pendingCertificate != null || isLoading) return

        pendingCertificate = PendingNodeCertificate.Discovered(candidate)
    }

    fun toggleNodeScan() {
        if (discoveredNodeAttempt != null) return

        scope.launch {
            if (nodeScanner.state is NodeScannerState.Scanning) {
                nodeScanner.stop()
            } else {
                nodeScanner.start()
            }
        }
    }

    pendingCertificate?.let { pending ->
        val certificate = pending.certificate

        AlertDialog(
            onDismissRequest = { pendingCertificate = null },
            title = { Text(certificateTitle) },
            text = {
                Column(verticalArrangement = Arrangement.spacedBy(MaterialSpacing.small)) {
                    Text(certificateMessage)
                    Text(
                        text = certificate.display,
                        style = MaterialTheme.typography.bodySmall,
                        fontFamily = FontFamily.Monospace,
                    )
                }
            },
            confirmButton = {
                TextButton(onClick = {
                    pendingCertificate = null

                    when (pending) {
                        is PendingNodeCertificate.Custom -> {
                            checkAndSaveCustomNode(TlsTrust.PinnedFingerprint(certificate.sha256))
                        }

                        is PendingNodeCertificate.Discovered -> {
                            connectToDiscoveredNode(pending.candidate)
                        }
                    }
                }) { Text(certificateTrust) }
            },
            dismissButton = {
                TextButton(onClick = { pendingCertificate = null }) { Text(certificateCancel) }
            },
        )
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
                            onClick = {
                                selectedNodeName = customElectrum
                            },
                        )

                        MaterialDivider()

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

                NodeScanSectionUi.Section(
                    state = nodeScanner.state,
                    selectedNodeUrl = selectedNodeSelection.toNode().url,
                    attempt = discoveredNodeAttempt,
                    actionsEnabled =
                        discoveredNodeAttempt == null &&
                            pendingCertificate == null &&
                            !isLoading,
                    callbacks =
                        NodeScanCallbacks(
                            toggleScan = ::toggleNodeScan,
                            selectVerified = ::connectToDiscoveredNode,
                            selectTrustRequired = ::presentDiscoveredCertificate,
                        ),
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

private object NodeScanSectionUi {
    @Composable
    fun Section(
        state: NodeScannerState,
        selectedNodeUrl: String,
        attempt: DiscoveredNodeAttempt?,
        actionsEnabled: Boolean,
        callbacks: NodeScanCallbacks,
    ) {
        val isScanning = state is NodeScannerState.Scanning

        Spacer(modifier = Modifier.height(MaterialSpacing.medium))
        MaterialDivider()

        NodeScanSectionHeader(
            isScanning = isScanning,
            enabled = actionsEnabled,
            onToggleScan = callbacks.toggleScan,
        )

        MaterialSection {
            Column(modifier = Modifier.fillMaxWidth()) {
                nodeScanProgress(state)?.let { NodeScanProgressRow(it) }
                NodeScanStatus(state)
                NodeScanResults(
                    results = nodeScanResults(state),
                    selectedNodeUrl = selectedNodeUrl,
                    attempt = attempt,
                    enabled = actionsEnabled,
                    callbacks = callbacks,
                )
            }
        }
    }

    @Composable
    private fun NodeScanSectionHeader(
        isScanning: Boolean,
        enabled: Boolean,
        onToggleScan: () -> Unit,
    ) {
        Row(
            modifier =
                Modifier
                    .fillMaxWidth()
                    .padding(start = MaterialSpacing.medium, end = MaterialSpacing.small),
            horizontalArrangement = Arrangement.SpaceBetween,
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Text(
                text = stringResource(R.string.node_scan_section_title),
                style = MaterialTheme.typography.labelMedium,
                color = MaterialTheme.colorScheme.primary,
            )

            TextButton(
                onClick = onToggleScan,
                enabled = enabled,
            ) {
                Text(
                    if (isScanning) {
                        stringResource(R.string.node_scan_stop)
                    } else {
                        stringResource(R.string.node_scan_rescan)
                    },
                )
            }
        }
    }

    @Composable
    private fun NodeScanResults(
        results: List<NodeScannerResult>,
        selectedNodeUrl: String,
        attempt: DiscoveredNodeAttempt?,
        enabled: Boolean,
        callbacks: NodeScanCallbacks,
    ) {
        results.forEachIndexed { index, result ->
            if (index > 0) {
                MaterialDivider()
            }

            when (result) {
                is NodeScannerResult.Verified -> {
                    DiscoveredNodeRows.VerifiedNodeRow(
                        node = result.v1,
                        isSelected = selectedNodeUrl == DiscoveredNodeRows.foundNodeUrl(result.v1),
                        isConnecting = attempt == DiscoveredNodeAttempt.Verified(result.v1),
                        enabled = enabled,
                        onClick = { callbacks.selectVerified(result.v1) },
                    )
                }

                is NodeScannerResult.TrustRequired -> {
                    DiscoveredNodeRows.TrustRequiredNodeRow(
                        candidate = result.v1,
                        isConnecting = attempt == DiscoveredNodeAttempt.TrustRequired(result.v1),
                        enabled = enabled,
                        onClick = { callbacks.selectTrustRequired(result.v1) },
                    )
                }
            }
        }
    }

    private fun nodeScanProgress(state: NodeScannerState): NodeScanProgress? =
        when (state) {
            is NodeScannerState.Scanning -> state.progress

            NodeScannerState.Idle,
            is NodeScannerState.Stopped,
            is NodeScannerState.Finished,
            is NodeScannerState.Failed,
            -> null
        }

    private fun nodeScanResults(state: NodeScannerState): List<NodeScannerResult> =
        when (state) {
            NodeScannerState.Idle -> emptyList()
            is NodeScannerState.Scanning -> state.results
            is NodeScannerState.Stopped -> state.results
            is NodeScannerState.Finished -> state.results
            is NodeScannerState.Failed -> state.results
        }

    @Composable
    private fun NodeScanProgressRow(progress: NodeScanProgress) {
        val progressFraction =
            if (progress.totalHosts == 0u) {
                0f
            } else {
                (progress.checkedHosts.toFloat() / progress.totalHosts.toFloat()).coerceIn(0f, 1f)
            }

        Column(
            modifier =
                Modifier
                    .fillMaxWidth()
                    .padding(horizontal = MaterialSpacing.medium, vertical = MaterialSpacing.small),
            verticalArrangement = Arrangement.spacedBy(MaterialSpacing.small),
        ) {
            LinearProgressIndicator(
                progress = { progressFraction },
                modifier = Modifier.fillMaxWidth(),
            )

            Text(
                text =
                    stringResource(
                        R.string.node_scan_progress,
                        progress.checkedHosts.toLong(),
                        progress.totalHosts.toLong(),
                    ),
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
    }

    @Composable
    private fun NodeScanStatus(state: NodeScannerState) {
        when (state) {
            NodeScannerState.Idle -> {
                NodeScanStatusText(stringResource(R.string.node_scan_description))
            }

            is NodeScannerState.Scanning -> {
                Unit
            }

            is NodeScannerState.Stopped -> {
                if (state.results.isEmpty()) {
                    NodeScanStatusText(stringResource(R.string.node_scan_stopped_empty))
                }
            }

            is NodeScannerState.Finished -> {
                if (state.results.isEmpty()) {
                    NodeScanStatusText(stringResource(R.string.node_scan_empty))
                }
            }

            is NodeScannerState.Failed -> {
                val message =
                    when (state.error) {
                        is NodeScannerException.NoLocalNetwork -> {
                            stringResource(R.string.node_scan_no_local_network)
                        }

                        is NodeScannerException.LocalNetworkPermissionDenied -> {
                            stringResource(R.string.node_scan_permission_denied)
                        }

                        is NodeScannerException.ScanFailed -> {
                            stringResource(R.string.node_scan_failed)
                        }

                        is NodeScannerException.ResultUnavailable -> {
                            stringResource(R.string.node_scan_results_unavailable)
                        }

                        is NodeScannerException.NetworkChanged -> {
                            stringResource(R.string.node_scan_network_changed)
                        }
                    }

                NodeScanStatusText(message)
            }
        }
    }

    @Composable
    private fun NodeScanStatusText(text: String) {
        Text(
            text = text,
            style = MaterialTheme.typography.bodyMedium,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            modifier =
                Modifier
                    .fillMaxWidth()
                    .padding(MaterialSpacing.medium),
        )
    }
}

private object DiscoveredNodeRows {
    @Composable
    fun VerifiedNodeRow(
        node: FoundNode,
        isSelected: Boolean,
        isConnecting: Boolean,
        enabled: Boolean,
        onClick: () -> Unit,
    ) {
        val software =
            if (node.serverSoftware.isBlank()) {
                stringResource(R.string.node_scan_electrum_server)
            } else {
                node.serverSoftware.trim()
            }
        val transport = foundNodeTransportLabel(node.transport)
        val endpoint = stringResource(R.string.node_scan_endpoint, node.ip, node.port.toInt(), transport)
        val accessibilityLabel =
            if (isSelected) {
                stringResource(R.string.node_scan_verified_selected_accessibility, software, endpoint)
            } else {
                stringResource(R.string.node_scan_verified_accessibility, software, endpoint)
            }

        Row(
            modifier =
                Modifier
                    .fillMaxWidth()
                    .clickable(
                        enabled = enabled,
                        onClickLabel = stringResource(R.string.node_scan_connect_hint),
                        role = Role.Button,
                        onClick = onClick,
                    ).semantics(mergeDescendants = true) {
                        contentDescription = accessibilityLabel
                    }.padding(horizontal = MaterialSpacing.medium, vertical = 12.dp),
            horizontalArrangement = Arrangement.spacedBy(MaterialSpacing.medium),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            VerifiedNodeDetails(
                software = software,
                endpoint = endpoint,
                isSelected = isSelected,
                modifier = Modifier.weight(1f),
            )

            if (isConnecting) {
                CircularProgressIndicator(modifier = Modifier.width(24.dp).height(24.dp))
            } else if (isSelected) {
                Icon(
                    imageVector = Icons.Default.Check,
                    contentDescription = null,
                    tint = MaterialTheme.colorScheme.primary,
                )
            }
        }
    }

    @Composable
    private fun VerifiedNodeDetails(
        software: String,
        endpoint: String,
        isSelected: Boolean,
        modifier: Modifier = Modifier,
    ) {
        Column(
            modifier = modifier,
            verticalArrangement = Arrangement.spacedBy(4.dp),
        ) {
            Text(
                text = software,
                style = MaterialTheme.typography.bodyMedium,
            )

            Text(
                text = endpoint,
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )

            Row(horizontalArrangement = Arrangement.spacedBy(MaterialSpacing.small)) {
                Text(
                    text = stringResource(R.string.node_scan_verified),
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.primary,
                )

                if (isSelected) {
                    Text(
                        text = stringResource(R.string.node_scan_selected),
                        style = MaterialTheme.typography.bodySmall,
                        fontWeight = FontWeight.SemiBold,
                        color = MaterialTheme.colorScheme.primary,
                    )
                }
            }
        }
    }

    @Composable
    fun TrustRequiredNodeRow(
        candidate: TrustRequiredEndpoint,
        isConnecting: Boolean,
        enabled: Boolean,
        onClick: () -> Unit,
    ) {
        val endpoint =
            stringResource(
                R.string.node_scan_endpoint,
                candidate.ip,
                candidate.port.toInt(),
                stringResource(R.string.node_scan_transport_tls),
            )
        val accessibilityLabel = stringResource(R.string.node_scan_trust_accessibility, endpoint)

        Row(
            modifier =
                Modifier
                    .fillMaxWidth()
                    .clickable(
                        enabled = enabled,
                        onClickLabel = stringResource(R.string.node_scan_trust_hint),
                        role = Role.Button,
                        onClick = onClick,
                    ).semantics(mergeDescendants = true) {
                        contentDescription = accessibilityLabel
                    }.padding(horizontal = MaterialSpacing.medium, vertical = 12.dp),
            horizontalArrangement = Arrangement.spacedBy(MaterialSpacing.medium),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            TrustRequiredNodeDetails(
                endpoint = endpoint,
                modifier = Modifier.weight(1f),
            )

            if (isConnecting) {
                CircularProgressIndicator(modifier = Modifier.width(24.dp).height(24.dp))
            }
        }
    }

    @Composable
    private fun TrustRequiredNodeDetails(
        endpoint: String,
        modifier: Modifier = Modifier,
    ) {
        Column(
            modifier = modifier,
            verticalArrangement = Arrangement.spacedBy(4.dp),
        ) {
            Text(
                text = stringResource(R.string.node_scan_trust_required),
                style = MaterialTheme.typography.bodyMedium,
                fontWeight = FontWeight.SemiBold,
                color = MaterialTheme.colorScheme.error,
            )

            Text(
                text = endpoint,
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )

            Text(
                text = stringResource(R.string.node_scan_unverified_endpoint),
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.error,
            )
        }
    }

    fun foundNodeUrl(node: FoundNode): String {
        val scheme =
            when (node.transport) {
                FoundNodeTransport.TCP -> "tcp"
                FoundNodeTransport.TLS -> "ssl"
            }

        return "$scheme://${node.ip}:${node.port}"
    }

    @Composable
    private fun foundNodeTransportLabel(transport: FoundNodeTransport): String =
        when (transport) {
            FoundNodeTransport.TCP -> stringResource(R.string.node_scan_transport_tcp)
            FoundNodeTransport.TLS -> stringResource(R.string.node_scan_transport_tls)
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
