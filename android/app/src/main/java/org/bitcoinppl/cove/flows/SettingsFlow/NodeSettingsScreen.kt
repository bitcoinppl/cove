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
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Scaffold
import androidx.compose.material3.SnackbarHost
import androidx.compose.material3.SnackbarHostState
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
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.input.KeyboardCapitalization
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.unit.dp
import java.util.Locale
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import org.bitcoinppl.cove.R
import org.bitcoinppl.cove.ui.theme.MaterialSpacing
import org.bitcoinppl.cove.views.MaterialDivider
import org.bitcoinppl.cove.views.MaterialSection
import org.bitcoinppl.cove.views.SectionHeader
import org.bitcoinppl.cove_core.ApiType
import org.bitcoinppl.cove_core.CertificateDecision
import org.bitcoinppl.cove_core.EndpointCertificateTrust
import org.bitcoinppl.cove_core.InternalException
import org.bitcoinppl.cove_core.Node
import org.bitcoinppl.cove_core.NodeCertificate
import org.bitcoinppl.cove_core.NodeSelection
import org.bitcoinppl.cove_core.NodeSelector
import org.bitcoinppl.cove_core.NodeSelectorException
import org.bitcoinppl.cove_core.TlsTrust

private data class CustomNodeInput(
    val selectedName: String,
    val url: String,
    val enteredName: String,
    val certificateTrust: EndpointCertificateTrust?,
)

private data class CustomNodeRequest(
    val requestedSelectionName: String,
    val node: Node,
)

private data class PendingCertificate(
    val request: CustomNodeRequest,
    val certificate: NodeCertificate,
)

private fun sameTlsTrust(left: TlsTrust?, right: TlsTrust?): Boolean = when {
    left is TlsTrust.CustomCa && right is TlsTrust.CustomCa -> left.cert.contentEquals(right.cert)
    left is TlsTrust.PinnedFingerprint && right is TlsTrust.PinnedFingerprint ->
        left.sha256.contentEquals(right.sha256)
    else -> left == right
}

private fun sameEndpointCertificateTrust(
    left: EndpointCertificateTrust?,
    right: EndpointCertificateTrust?,
): Boolean = when {
    left == null || right == null -> left == right
    else -> left.endpoint == right.endpoint && sameTlsTrust(left.tls, right.tls)
}

private fun sameCustomNodeInput(left: CustomNodeInput, right: CustomNodeInput): Boolean =
        left.selectedName == right.selectedName &&
        left.url == right.url &&
        left.enteredName == right.enteredName &&
        sameEndpointCertificateTrust(left.certificateTrust, right.certificateTrust)

private fun sameNodeIdentity(left: Node, right: Node): Boolean =
    left.name == right.name &&
        left.network == right.network &&
        left.apiType == right.apiType &&
        left.url == right.url &&
        sameTlsTrust(left.tls, right.tls)

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun NodeSettingsScreen(
    app: org.bitcoinppl.cove.AppManager,
    modifier: Modifier = Modifier,
) {
    val nodeSelector = remember { NodeSelector() }
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
    var pendingCertificate by remember { mutableStateOf<PendingCertificate?>(null) }
    // a certificate accepted in this session, paired with its endpoint. Rust
    // decides whether the endpoint matches the current request
    var customCertificateTrust by remember { mutableStateOf<EndpointCertificateTrust?>(null) }
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

    val showCustomFields =
        selectedNodeSelection is NodeSelection.Custom ||
            selectedNodeName == customElectrum ||
            selectedNodeName == customEsplora

    // pull a fresh snapshot because NodeSelector mutates persisted node settings directly
    fun refreshNodeState() {
        NodeSelector().use { refreshedNodeSelector ->
            nodeList = refreshedNodeSelector.nodeList()
            selectedNodeSelection = refreshedNodeSelector.selectedNode()
        }
        selectedNodeName = selectedNodeSelection.toNode().name
    }

    fun restoreStoredNodeForm() {
        val storedSelection = nodeSelector.selectedNode()
        val storedNode = storedSelection.toNode()

        selectedNodeSelection = storedSelection
        selectedNodeName = storedNode.name
        customCertificateTrust = storedNode.tls?.let { tls ->
            EndpointCertificateTrust(storedNode.url, tls)
        }

        if (storedSelection is NodeSelection.Custom) {
            customUrl = storedNode.url
            customNodeName = storedNode.name
        } else {
            customUrl = ""
            customNodeName = ""
        }
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
                    customCertificateTrust = node.tls?.let { tls ->
                        EndpointCertificateTrust(node.url, tls)
                    }
                }
            }
        }
    }

    fun selectPresetNode(nodeName: String) {
        if (isLoading) return

        if (selectedNodeSelection is NodeSelection.Preset && selectedNodeName == nodeName) {
            return
        }

        selectedNodeName = nodeName
        customUrl = ""
        customNodeName = ""
        customCertificateTrust = null

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
            } catch (e: Exception) {
                restoreStoredNodeForm()

                when (e) {
                    is NodeSelectorException.NodeNotFound -> {
                        errorTitle = errorTitleDefault
                        errorMessage = errorNotFound.format(e.v1)
                    }
                    is NodeSelectorException.NodeAccessException -> {
                        errorTitle = errorConnectionFailed
                        errorMessage = errorConnectionMessage.format(e.v1)
                    }
                    else -> {
                        errorTitle = errorTitleDefault
                        errorMessage = errorUnknown.format(e.message ?: "")
                    }
                }

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

    fun currentCustomNodeInput(): CustomNodeInput {
        return CustomNodeInput(
            selectedName = selectedNodeName,
            url = customUrl,
            enteredName = customNodeName,
            certificateTrust = customCertificateTrust,
        )
    }

    fun isCurrentCustomInput(input: CustomNodeInput): Boolean =
        sameCustomNodeInput(currentCustomNodeInput(), input)

    // Whether the certificate can be offered for confirmation is decided in the
    // core, so both apps apply the same rule.
    fun isCurrentCustomRequest(request: CustomNodeRequest): Boolean {
        if (selectedNodeName != request.requestedSelectionName) return false

        return try {
            val currentNode =
                nodeSelector.parseCustomNode(
                    customUrl,
                    selectedNodeName,
                    customNodeName,
                    customCertificateTrust,
                )

            sameNodeIdentity(currentNode, request.node)
        } catch (_: NodeSelectorException) {
            false
        } catch (_: InternalException) {
            false
        }
    }

    suspend fun offerCertificate(request: CustomNodeRequest) {
        try {
            val decision =
                withContext(Dispatchers.IO) {
                    nodeSelector.certificateDecision(request.node.url)
                }

            if (!isCurrentCustomRequest(request)) return

            when (decision) {
                is CertificateDecision.Unrecognized -> {
                    pendingCertificate = PendingCertificate(request, decision.certificate)
                }
                is CertificateDecision.Changed -> {
                    errorTitle = certificateChangedTitle
                    errorMessage = certificateChangedMessage
                    showErrorDialog = true
                }
            }
        } catch (readError: NodeSelectorException) {
            if (isCurrentCustomRequest(request)) {
                showCertificateReadError(readError.message)
            }
        } catch (readError: InternalException) {
            if (isCurrentCustomRequest(request)) {
                showCertificateReadError(readError.message)
            }
        }
    }

    suspend fun handleCustomNodeError(
        error: Exception,
        input: CustomNodeInput,
        request: CustomNodeRequest?,
    ) {
        when (error) {
            is NodeSelectorException.ParseNodeUrlException -> {
                if (isCurrentCustomInput(input)) {
                    errorTitle = errorParseTitle
                    errorMessage = error.v1
                    showErrorDialog = true
                }
            }
            is NodeSelectorException.NodeAccessException -> {
                if (request != null && isCurrentCustomRequest(request)) {
                    errorTitle = errorConnectionFailed
                    errorMessage = errorConnectionMessage.format(error.v1)
                    showErrorDialog = true
                }
            }
            is NodeSelectorException.CertificateNotTrusted -> {
                android.util.Log.d("NodeSettings", "certificate not trusted, offering it", error)
                val currentRequest = request ?: return
                if (isCurrentCustomRequest(currentRequest)) {
                    offerCertificate(currentRequest)
                }
            }
            else -> {
                val isCurrent =
                    request?.let(::isCurrentCustomRequest) ?: isCurrentCustomInput(input)
                if (isCurrent) {
                    errorTitle = errorTitleDefault
                    errorMessage = String.format(Locale.US, errorUnknown, error.message.orEmpty())
                    showErrorDialog = true
                }
            }
        }
    }

    fun checkAndSaveCustomNode() {
        if (isLoading) return

        if (customUrl.isEmpty()) {
            errorTitle = errorTitleDefault
            errorMessage = errorUrlEmpty
            showErrorDialog = true
            return
        }

        val requestedSelectionName = selectedNodeName
        val requestedCustomUrl = customUrl
        val requestedCustomNodeName = customNodeName
        val requestedCertificateTrust = customCertificateTrust
        val input =
            CustomNodeInput(
                selectedName = requestedSelectionName,
                url = requestedCustomUrl,
                enteredName = requestedCustomNodeName,
                certificateTrust = requestedCertificateTrust,
            )

        isLoading = true
        scope.launch {
            var request: CustomNodeRequest? = null

            try {
                val node =
                    withContext(Dispatchers.IO) {
                        nodeSelector.parseCustomNode(
                            input.url,
                            input.selectedName,
                            input.enteredName,
                            input.certificateTrust,
                        )
                    }

                if (!isCurrentCustomInput(input)) return@launch

                request = CustomNodeRequest(requestedSelectionName, node)

                // update fields with parsed values
                customUrl = node.url
                customNodeName = node.name

                withContext(Dispatchers.IO) {
                    nodeSelector.checkNode(node)
                }

                val currentRequest = request
                if (!isCurrentCustomRequest(currentRequest)) return@launch

                nodeSelector.saveNode(node)
                refreshNodeState()

                // launch snackbar in separate coroutine so it doesn't block finally
                scope.launch {
                    snackbarHostState.showSnackbar(successSaved)
                }
            } catch (e: Exception) {
                handleCustomNodeError(e, input, request)
            } finally {
                isLoading = false
            }
        }
    }

    pendingCertificate?.let { pending ->
        AlertDialog(
            onDismissRequest = { pendingCertificate = null },
            title = { Text(certificateTitle) },
            text = {
                Column(verticalArrangement = Arrangement.spacedBy(MaterialSpacing.small)) {
                    Text(certificateMessage)
                    Text(
                        text = pending.certificate.display,
                        style = MaterialTheme.typography.bodySmall,
                        fontFamily = FontFamily.Monospace,
                    )
                }
            },
            confirmButton = {
                TextButton(onClick = {
                    if (isCurrentCustomRequest(pending.request)) {
                        pendingCertificate = null
                        customCertificateTrust = EndpointCertificateTrust(
                            pending.request.node.url,
                            TlsTrust.PinnedFingerprint(pending.certificate.sha256),
                        )
                        customUrl = pending.request.node.url
                        checkAndSaveCustomNode()
                    } else {
                        pendingCertificate = null
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
                                enabled = !isLoading,
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
                            enabled = !isLoading,
                        )

                        MaterialDivider()

                        // custom esplora
                        NodeRow(
                            nodeName = customEsplora,
                            isSelected = selectedNodeName == customEsplora,
                            onClick = {
                                selectedNodeName = customEsplora
                            },
                            enabled = !isLoading,
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
                                onValueChange = { if (!isLoading) customUrl = it },
                                enabled = !isLoading,
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
                                onValueChange = { if (!isLoading) customNodeName = it },
                                enabled = !isLoading,
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
    enabled: Boolean,
) {
    Row(
        modifier =
            Modifier
                .fillMaxWidth()
                .clickable(enabled = enabled, onClick = onClick)
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
