package org.bitcoinppl.cove.flows.SettingsFlow

import androidx.compose.foundation.background
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
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.safeDrawing
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.filled.Check
import androidx.compose.material.icons.filled.CheckCircle
import androidx.compose.material.icons.filled.Close
import androidx.compose.material.icons.filled.FiberManualRecord
import androidx.compose.material.icons.filled.Lock
import androidx.compose.material.icons.filled.Refresh
import androidx.compose.material.icons.filled.Visibility
import androidx.compose.material.icons.filled.Public
import androidx.compose.material.icons.filled.Terminal
import androidx.compose.material.icons.filled.Settings
import androidx.compose.material.icons.filled.Info
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.Button
import androidx.compose.material3.IconButton
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.RadioButton
import androidx.compose.material3.Scaffold
import androidx.compose.material3.SnackbarHost
import androidx.compose.material3.SnackbarHostState
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.material3.rememberModalBottomSheetState
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalClipboardManager
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import org.bitcoinppl.cove.R
import org.bitcoinppl.cove.views.MaterialDivider
import org.bitcoinppl.cove.views.MaterialSection
import org.bitcoinppl.cove.views.MaterialSettingsItem
import org.bitcoinppl.cove.views.SectionHeader
import org.bitcoinppl.cove_core.ApiType
import org.bitcoinppl.cove_core.AppAction
import org.bitcoinppl.cove_core.Database
import org.bitcoinppl.cove_core.GlobalConfigKey
import org.bitcoinppl.cove_core.GlobalFlagKey
import org.bitcoinppl.cove_core.Node
import org.bitcoinppl.cove_core.NodeSelector
import org.bitcoinppl.cove_core.NodeSelectorException
import org.bitcoinppl.cove_core.TorMode as CoreTorMode
import org.bitcoinppl.cove_core.builtInTorBootstrapStatus
import org.bitcoinppl.cove_core.ensureBuiltInTorBootstrap
import org.bitcoinppl.cove_core.torConnectionLogs
import org.bitcoinppl.cove_core.types.Network
import org.bitcoinppl.cove_core.types.allNetworks
import org.bitcoinppl.cove.Log
import org.bitcoinppl.cove.ui.theme.CoveColor
import org.bitcoinppl.cove.tor.deriveBuiltInBootstrapSnapshot
import org.bitcoinppl.cove.tor.parseCoreTorMode
import org.bitcoinppl.cove.tor.testSocksEndpoint
import org.bitcoinppl.cove.tor.testTorApiThroughSocks
import java.net.SocketTimeoutException
import java.time.LocalTime

private enum class TorTestStepStatus {
    Pending,
    Running,
    Passed,
    Failed,
}

private data class TorTestStep(
    val key: String,
    val title: String,
    val detail: String,
    val status: TorTestStepStatus = TorTestStepStatus.Pending,
)

private data class TorConnectionTestState(
    val running: Boolean = false,
    val finished: Boolean = false,
    val steps: List<TorTestStep> = emptyList(),
    val logs: List<String> = emptyList(),
)

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun NetworkSettingsScreen(
    app: org.bitcoinppl.cove.AppManager,
    modifier: Modifier = Modifier,
) {
    val logTag = "NetworkSettingsScreen"
    val context = LocalContext.current
    val clipboardManager = LocalClipboardManager.current
    val networks = remember { allNetworks() }
    val selectedNetwork = app.selectedNetwork
    var pendingNetworkChange by remember { mutableStateOf<Network?>(null) }

    val database = remember { Database() }
    val globalConfig = remember { database.globalConfig() }
    val globalFlag = remember { database.globalFlag() }
    val nodeSelector = remember { NodeSelector() }
    val scope = rememberCoroutineScope()
    val snackbarHostState = remember { SnackbarHostState() }
    val persistedUseTor = remember { runCatching { globalConfig.useTor() }.getOrDefault(false) }
    val torSettingsDiscovered = remember {
        persistedUseTor ||
            runCatching {
                globalFlag.getBoolConfig(GlobalFlagKey.TOR_SETTINGS_DISCOVERED)
            }.getOrDefault(false)
    }
    val persistedTorMode = remember {
        parseTorMode(runCatching { globalConfig.get(GlobalConfigKey.TorMode) }.getOrNull())
    }
    val persistedExternalHost = remember {
        runCatching { globalConfig.get(GlobalConfigKey.TorExternalHost) }.getOrNull()
            ?.takeIf { it.isNotBlank() }
            ?: "127.0.0.1"
    }
    val persistedExternalPort = remember {
        runCatching { globalConfig.torExternalPort() }.getOrDefault(9050).toString()
    }

    var uiState by
        remember {
            mutableStateOf(
                TorUiState(
                    enabled = persistedUseTor,
                    mode = persistedTorMode,
                    externalHost = persistedExternalHost,
                    externalPort = persistedExternalPort,
                    externalValidationError = validateExternalConfig(persistedExternalHost, persistedExternalPort),
                ),
            )
        }
    var showFullLogDialog by remember { mutableStateOf(false) }
    var showModeDialog by remember { mutableStateOf(false) }
    var showTorTestDialog by remember { mutableStateOf(false) }
    var showDisableTorOnionDialog by remember { mutableStateOf(false) }
    var rustTorLogCount by remember { mutableStateOf(0) }
    var builtInWarmupRequested by remember { mutableStateOf(false) }
    var torTestState by remember { mutableStateOf(TorConnectionTestState()) }
    var autoPendingTestKey by remember { mutableStateOf<String?>(null) }

    fun appendTorLog(message: String) {
        val timestamp = LocalTime.now().withNano(0)
        val entry = "[$timestamp] $message"
        val merged = (uiState.logLines + entry).takeLast(300)
        uiState = uiState.copy(latestLogLine = message, logLines = merged)
    }

    fun syncRustTorLogs() {
        if (!uiState.enabled || uiState.mode != TorMode.BuiltIn) {
            return
        }

        runCatching { torConnectionLogs() }
            .onSuccess { rustLogs ->
                if (rustLogs.size < rustTorLogCount) {
                    rustTorLogCount = 0
                }

                val newLogs = rustLogs.drop(rustTorLogCount)
                rustTorLogCount = rustLogs.size

                val merged =
                    if (newLogs.isEmpty()) {
                        uiState.logLines
                    } else {
                        (uiState.logLines + newLogs).takeLast(300)
                    }
                val snapshot = deriveBuiltInBootstrapSnapshot(rustLogs)
                val structuredStatus = runCatching { builtInTorBootstrapStatus() }.getOrNull()
                val hasStructuredStatus = structuredStatus?.launched == true
                val status =
                    when {
                        !uiState.enabled -> TorStatus.Disabled
                        structuredStatus?.ready == true -> TorStatus.Ready
                        structuredStatus?.lastError != null -> TorStatus.Error
                        hasStructuredStatus -> TorStatus.Bootstrapping
                        snapshot.isReady -> TorStatus.Ready
                        snapshot.hasError -> TorStatus.Error
                        else -> TorStatus.Bootstrapping
                    }
                val currentStep =
                    structuredStatus
                        ?.lastError
                        ?: structuredStatus
                            ?.blocked
                            ?.let { "Blocked: $it" }
                        ?: snapshot
                            .step
                            .takeIf { Regex("""^\d{1,3}%:""").containsMatchIn(it) }
                        ?: structuredStatus
                            ?.message
                            ?.takeIf { hasStructuredStatus && it.isNotBlank() }
                        ?: snapshot.step
                val messagePercent =
                    Regex("""^(\d{1,3})%:""")
                        .find(currentStep)
                        ?.groupValues
                        ?.getOrNull(1)
                        ?.toIntOrNull()
                        ?.coerceIn(0, 100)
                val structuredPercent =
                    structuredStatus
                        ?.takeIf { hasStructuredStatus }
                        ?.percent
                        ?.toInt()
                        ?.coerceIn(0, 100)
                val progressPercent =
                    when {
                        status == TorStatus.Ready -> 100
                        else -> messagePercent ?: structuredPercent ?: snapshot.percent
                    }

                uiState =
                    uiState.copy(
                        status = status,
                        currentStep = currentStep,
                        latestLogLine = snapshot.lastLine,
                        logLines = merged,
                        progressPercent = progressPercent,
                    )
            }.onFailure { error ->
                Log.e(logTag, "syncRustTorLogs failed: ${error.message}", error)
            }
    }

    fun clearPendingOnionDraft() {
        app.pendingNodeAwaitingTorSetup = false
        app.pendingNodeTorValidated = false
        app.pendingNodeUrl = ""
        app.pendingNodeName = ""
        app.pendingNodeTypeName = ""
        autoPendingTestKey = null
    }

    fun disableTorState() {
        runCatching {
            globalConfig.setUseTor(false)
        }.onFailure { error ->
            appendTorLog("Failed to disable Tor: ${error.message ?: "unknown error"}")
            scope.launch {
                snackbarHostState.showSnackbar("Could not disable Tor: ${error.message ?: "unknown error"}")
            }
            return
        }

        uiState = uiState.copy(enabled = false)
        builtInWarmupRequested = false
    }

    fun enableTorState(): Boolean =
        runCatching {
            globalConfig.setUseTor(true)
        }.onFailure { error ->
            appendTorLog("Failed to enable Tor: ${error.message ?: "unknown error"}")
            scope.launch {
                snackbarHostState.showSnackbar("Could not enable Tor: ${error.message ?: "unknown error"}")
            }
        }.isSuccess

    fun setPersistedTorMode(
        mode: TorMode,
        onFailure: (Throwable) -> Unit = {},
    ): Boolean {
        val persistedMode =
            when (mode) {
                TorMode.BuiltIn -> CoreTorMode.BUILT_IN.name
                TorMode.Orbot -> CoreTorMode.ORBOT.name
                TorMode.External -> CoreTorMode.EXTERNAL.name
            }

        return runCatching {
            globalConfig.set(GlobalConfigKey.TorMode, persistedMode)
        }.onFailure { error ->
            appendTorLog("Failed to save Tor mode: ${error.message ?: "unknown error"}")
            scope.launch {
                snackbarHostState.showSnackbar("Could not save Tor mode: ${error.message ?: "unknown error"}")
            }
            onFailure(error)
        }.isSuccess
    }

    fun saveExternalTorConfig(
        host: String,
        port: UShort,
        proxyLog: String = redactedProxyForLog(host, port),
    ): Boolean =
        runCatching {
            globalConfig.set(GlobalConfigKey.TorExternalHost, host)
            globalConfig.setTorExternalPort(port)
        }.onFailure { error ->
            appendTorLog("Failed to save external Tor config $proxyLog: ${error.message ?: "unknown error"}")
            scope.launch {
                snackbarHostState.showSnackbar("Could not save Tor proxy configuration: ${error.message ?: "unknown error"}")
            }
        }.isSuccess

    fun persistTorTestConfiguration(host: String, port: UShort): Boolean {
        if (!setPersistedTorMode(uiState.mode)) {
            return false
        }
        if (uiState.mode == TorMode.External && !saveExternalTorConfig(host, port)) {
            return false
        }
        return enableTorState()
    }

    suspend fun testTorProxy(host: String, port: UShort): Result<Unit> =
        try {
            val proxyLog = redactedProxyForLog(host, port)
            syncRustTorLogs()
            appendTorLog("Testing SOCKS endpoint $proxyLog")
            Log.d(logTag, "testTorProxy start: proxy=$proxyLog")
            testSocksEndpoint(host, port.toInt()).getOrThrow()
            Log.d(logTag, "testTorProxy success: proxy=$proxyLog")
            appendTorLog("SOCKS endpoint reachable: $proxyLog")
            syncRustTorLogs()
            Result.success(Unit)
        } catch (error: CancellationException) {
            throw error
        } catch (error: Exception) {
            val proxyLog = redactedProxyForLog(host, port)
            Log.e(logTag, "testTorProxy failed: proxy=$proxyLog, reason=${error.message}", error)
            appendTorLog("SOCKS endpoint failed: $proxyLog (${error.message ?: "unknown error"})")
            syncRustTorLogs()
            Result.failure(error)
        }

    suspend fun resolveNodeForTorTest(): Node {
        val (node, logMessage) =
            withContext(Dispatchers.IO) {
                if (app.pendingNodeAwaitingTorSetup && app.pendingNodeUrl.isNotBlank()) {
                    val typeName = app.pendingNodeTypeName.ifBlank { "Custom Electrum" }
                    Log.d(logTag, "resolveNodeForTorTest using pending node: type=$typeName, ${redactedEndpointForLog(app.pendingNodeUrl)}")
                    val parsed = nodeSelector.parseCustomNode(app.pendingNodeUrl, typeName, app.pendingNodeName)
                    parsed to "Using pending node for Tor test: ${redactedNodeForLog(parsed)}"
                } else {
                    val selected = app.selectedNode
                    Log.d(logTag, "resolveNodeForTorTest using selected node: ${redactedNodeForLog(selected)}")
                    selected to "Using selected node for Tor test: ${redactedNodeForLog(selected)}"
                }
            }

        appendTorLog(logMessage)
        return node
    }

    suspend fun runNodeTorTest(node: Node): Result<Unit> =
        try {
            syncRustTorLogs()
            val nodeLog = redactedNodeForLog(node)
            appendTorLog("Checking node via Tor: $nodeLog")
            Log.d(logTag, "runNodeTorTest start: $nodeLog")
            withContext(Dispatchers.IO) {
                nodeSelector.checkSelectedNode(node)
            }
            Log.d(logTag, "runNodeTorTest success: $nodeLog")
            appendTorLog("Node check passed: $nodeLog")
            syncRustTorLogs()
            Result.success(Unit)
        } catch (error: CancellationException) {
            throw error
        } catch (error: Exception) {
            val nodeLog = redactedNodeForLog(node)
            Log.e(logTag, "runNodeTorTest failed: $nodeLog, reason=${error.message}", error)
            appendTorLog("Node check failed: $nodeLog (${error.message ?: "unknown error"})")
            syncRustTorLogs()
            Result.failure(error)
        }

    fun appendTorTestLog(message: String) {
        torTestState = torTestState.copy(logs = (torTestState.logs + message).takeLast(150))
    }

    fun updateTorTestStep(
        stepKey: String,
        status: TorTestStepStatus,
        detail: String? = null,
    ) {
        torTestState =
            torTestState.copy(
                steps =
                    torTestState.steps.map { step ->
                        if (step.key != stepKey) {
                            step
                        } else {
                            step.copy(
                                status = status,
                                detail = detail ?: step.detail,
                            )
                        }
                    },
            )
    }

    fun parseEndpointHostPort(endpoint: String): Pair<String, UShort>? {
        val host = endpoint.substringBefore(':', "")
        val port = endpoint.substringAfter(':', "").toIntOrNull()
        if (host.isBlank() || port == null || port !in 1..65535) {
            return null
        }
        return host to port.toUShort()
    }

    suspend fun runTorApiTestWithRetries(
        host: String,
        port: Int,
    ): Result<org.bitcoinppl.cove.tor.TorApiSnapshot> {
        val maxAttempts = 5
        var timeoutMs = 15000
        var lastError: Throwable? = null

        repeat(maxAttempts) { index ->
            val attempt = index + 1
            appendTorTestLog("Tor API check attempt $attempt/$maxAttempts (timeout=${timeoutMs}ms)")

            val result = testTorApiThroughSocks(host, port, timeoutMs)
            if (result.isSuccess) {
                return result
            }

            val error = result.exceptionOrNull()
            lastError = error
            val timeoutLike =
                error is SocketTimeoutException ||
                    (error?.message?.lowercase()?.contains("timeout") == true) ||
                    (error?.message?.lowercase()?.contains("timed out") == true)

            if (!timeoutLike || attempt >= maxAttempts) {
                return Result.failure(error ?: IllegalStateException("unknown tor api error"))
            }

            val snapshot = deriveBuiltInBootstrapSnapshot(runCatching { torConnectionLogs() }.getOrDefault(emptyList()))
            appendTorTestLog(
                "Tor API timed out; retrying while Tor bootstraps (${snapshot.percent}% ${snapshot.step.lowercase()})",
            )
            syncRustTorLogs()
            delay((attempt * 1200L).coerceAtMost(6000L))
            timeoutMs = (timeoutMs + 4000).coerceAtMost(30000)
        }

        return Result.failure(lastError ?: IllegalStateException("unknown tor api error"))
    }

    suspend fun runProgressiveTorTest() {
        val endpoint: Pair<String, UShort> =
            try {
                when (uiState.mode) {
                    TorMode.BuiltIn -> {
                        val endpointValue = ensureBuiltInTorBootstrap()
                        parseEndpointHostPort(endpointValue)
                            ?: ("127.0.0.1" to 39050u.toUShort())
                    }
                    TorMode.Orbot -> "127.0.0.1" to 9050u.toUShort()
                    TorMode.External -> {
                        val validationError = validateExternalConfig(uiState.externalHost, uiState.externalPort)
                        if (validationError != null) {
                            uiState = uiState.copy(externalValidationError = validationError)
                            snackbarHostState.showSnackbar(validationError)
                            return
                        }
                        uiState.externalHost to (uiState.externalPort.toUShortOrNull() ?: 9050u.toUShort())
                    }
                }
            } catch (error: CancellationException) {
                throw error
            } catch (error: Exception) {
                app.pendingNodeTorValidated = false
                uiState = uiState.copy(status = TorStatus.Error)
                appendTorLog("Failed to prepare Tor endpoint: ${error.message ?: "unknown error"}")
                snackbarHostState.showSnackbar("Failed to prepare Tor endpoint: ${error.message ?: "unknown error"}")
                return
            }

        val (host, port) = endpoint
        if (!persistTorTestConfiguration(host, port)) {
            app.pendingNodeTorValidated = false
            uiState = uiState.copy(status = TorStatus.Error)
            return
        }

        val proxyLog = redactedProxyForLog(host, port)
        showTorTestDialog = true
        torTestState =
            TorConnectionTestState(
                running = true,
                finished = false,
                steps =
                    listOf(
                        TorTestStep(
                            key = "proxy",
                            title = "Tor proxy reachable",
                            detail = "Checking SOCKS endpoint $proxyLog",
                        ),
                        TorTestStep(
                            key = "api",
                            title = "Tor API reports Tor exit",
                            detail = "Checking torproject API over SOCKS",
                        ),
                        TorTestStep(
                            key = "node",
                            title = "Node reachable via Tor",
                            detail = "Checking selected node over Tor",
                        ),
                    ),
                logs = listOf("Starting Tor connection test (${uiState.mode})"),
            )

        when (uiState.mode) {
            TorMode.BuiltIn -> {
                if (!builtInWarmupRequested) {
                    builtInWarmupRequested = true
                    runCatching { ensureBuiltInTorBootstrap() }
                        .onSuccess { endpointValue ->
                            appendTorTestLog("Built-in Tor warmup requested at $endpointValue")
                        }.onFailure { error ->
                            appendTorTestLog("Built-in Tor warmup failed: ${error.message}")
                        }
                }
            }
            TorMode.Orbot,
            TorMode.External,
            -> Unit
        }

        updateTorTestStep("proxy", TorTestStepStatus.Running)
        val proxyResult = testTorProxy(host, port)
        if (proxyResult.isFailure) {
            val message = proxyResult.exceptionOrNull()?.message ?: "unknown error"
            updateTorTestStep("proxy", TorTestStepStatus.Failed, "SOCKS check failed: $message")
            appendTorTestLog("SOCKS check failed: $message")
            app.pendingNodeTorValidated = false
            uiState = uiState.copy(status = TorStatus.Error)
            torTestState = torTestState.copy(running = false, finished = true)
            snackbarHostState.showSnackbar("Tor proxy unavailable: $message")
            return
        }
        updateTorTestStep("proxy", TorTestStepStatus.Passed, "SOCKS endpoint reachable")
        appendTorTestLog("SOCKS endpoint reachable")

        updateTorTestStep("api", TorTestStepStatus.Running)
        val torApiResult = runTorApiTestWithRetries(host, port.toInt())
        if (torApiResult.isFailure) {
            val message = torApiResult.exceptionOrNull()?.message ?: "unknown error"
            updateTorTestStep("api", TorTestStepStatus.Failed, "Tor API request failed: $message")
            appendTorTestLog("Tor API request failed: $message")
            app.pendingNodeTorValidated = false
            uiState = uiState.copy(status = TorStatus.Error)
            torTestState = torTestState.copy(running = false, finished = true)
            snackbarHostState.showSnackbar("Tor API check failed: $message")
            return
        }

        val apiSnapshot = torApiResult.getOrThrow()
        if (!apiSnapshot.isTor) {
            updateTorTestStep("api", TorTestStepStatus.Failed, "Tor API response did not confirm Tor routing")
            appendTorTestLog("Tor API did not confirm Tor routing: ${apiSnapshot.raw}")
            app.pendingNodeTorValidated = false
            uiState = uiState.copy(status = TorStatus.Error)
            torTestState = torTestState.copy(running = false, finished = true)
            snackbarHostState.showSnackbar("Tor API check failed: traffic is not exiting through Tor")
            return
        }
        updateTorTestStep("api", TorTestStepStatus.Passed, "Tor API confirmed Tor routing${apiSnapshot.ip?.let { " ($it)" } ?: ""}")
        appendTorTestLog("Tor API confirmed Tor routing${apiSnapshot.ip?.let { " via $it" } ?: ""}")

        updateTorTestStep("node", TorTestStepStatus.Running)
        val node = resolveNodeForTorTest()
        val nodeResult = runNodeTorTest(node)
        if (nodeResult.isFailure) {
            val errorText = (nodeResult.exceptionOrNull() as? NodeSelectorException.NodeAccessException)?.v1
                ?: (nodeResult.exceptionOrNull()?.message ?: "unknown error")
            updateTorTestStep("node", TorTestStepStatus.Failed, "Node check failed: $errorText")
            appendTorTestLog("Node check failed: $errorText")
            app.pendingNodeTorValidated = false
            uiState = uiState.copy(status = TorStatus.Error)
            torTestState = torTestState.copy(running = false, finished = true)
            snackbarHostState.showSnackbar("Node test failed: $errorText")
            return
        }

        updateTorTestStep("node", TorTestStepStatus.Passed, "Node reachable via Tor")
        appendTorTestLog("Node reachable via Tor")
        app.pendingNodeTorValidated = true
        if (uiState.mode == TorMode.BuiltIn) {
            syncRustTorLogs()
        } else {
            uiState = uiState.copy(status = TorStatus.Ready, progressPercent = 100)
        }
        appendTorLog(
            when (uiState.mode) {
                TorMode.BuiltIn -> "Built-in Tor validation passed"
                TorMode.External -> "External Tor validation passed"
                TorMode.Orbot -> "Orbot Tor validation passed"
            },
        )
        if (app.pendingNodeAwaitingTorSetup) {
            appendTorTestLog("Pending onion node validated; applying configuration")
            appendTorLog("Pending onion node validated; applying configuration")
            app.popRoute()
        }
        torTestState = torTestState.copy(running = false, finished = true)
        snackbarHostState.showSnackbar("Tor connection test passed")
    }

    val statusDisabledText = stringResource(R.string.tor_status_disabled)
    val logDisabledText = stringResource(R.string.tor_log_disabled)
    val torStatusTesting = stringResource(R.string.tor_status_testing)
    val torStatusConfigured = stringResource(R.string.tor_status_configured)
    val torStatusInstallOrbot = stringResource(R.string.tor_status_install_orbot)
    val torStatusActionRequired = stringResource(R.string.tor_status_action_required)
    val torActionSaveConfig = stringResource(R.string.tor_action_save_config)
    val torActionTestConnection = stringResource(R.string.tor_action_test_connection)
    val torActionOpenOrbot = stringResource(R.string.tor_action_open_orbot)
    val torActionInstallOrbot = stringResource(R.string.tor_action_install_orbot)
    val torErrorConfigInvalid = stringResource(R.string.tor_error_config_invalid)
    val torSuccessConfigurationSaved = stringResource(R.string.tor_success_configuration_saved)
    val torSuccessConnectionValid = stringResource(R.string.tor_success_connection_valid)
    val torDisableOnionDialogTitle = stringResource(R.string.tor_disable_onion_dialog_title)
    val torDisableOnionDialogBody = stringResource(R.string.tor_disable_onion_dialog_body)
    val torDisableOnionDialogConfirm = stringResource(R.string.tor_disable_onion_dialog_confirm)
    val torDisableOnionDialogCancel = stringResource(R.string.tor_disable_onion_dialog_cancel)
    val torFallbackFailed = stringResource(R.string.tor_fallback_failed)
    val torFallbackApplied = stringResource(R.string.tor_fallback_applied)
    val copiedText = stringResource(R.string.btn_copied)

    LaunchedEffect(Unit) {
        rustTorLogCount = 0
        appendTorLog("Opened Tor connection logs")
        val (status, version) = OrbotPackageHelper.detect(context)
        uiState = uiState.copy(orbotStatus = status, orbotVersion = version)
        appendTorLog(
            when (status) {
                OrbotStatus.Detected -> "Orbot detected${version?.let { " ($it)" } ?: ""}"
                OrbotStatus.NotDetected -> "Orbot not detected"
                OrbotStatus.Checking -> "Checking Orbot status"
            },
        )
        if (uiState.enabled && uiState.mode == TorMode.BuiltIn && !builtInWarmupRequested) {
            builtInWarmupRequested = true
            runCatching { ensureBuiltInTorBootstrap() }
                .onSuccess { endpoint ->
                    appendTorLog("Built-in Tor bootstrap started at $endpoint")
                }.onFailure { error ->
                    appendTorLog("Built-in Tor bootstrap failed: ${error.message ?: "unknown error"}")
                }
        }
        syncRustTorLogs()
    }

    LaunchedEffect(uiState.enabled, uiState.mode) {
        if (!uiState.enabled || uiState.mode != TorMode.BuiltIn) {
            return@LaunchedEffect
        }

        if (!builtInWarmupRequested) {
            builtInWarmupRequested = true
            runCatching { ensureBuiltInTorBootstrap() }
                .onSuccess { endpoint ->
                    appendTorLog("Built-in Tor bootstrap started at $endpoint")
                }.onFailure { error ->
                    appendTorLog("Built-in Tor bootstrap failed: ${error.message ?: "unknown error"}")
                }
        }

        while (uiState.enabled && uiState.mode == TorMode.BuiltIn) {
            syncRustTorLogs()
            delay(1000)
        }
    }

    LaunchedEffect(uiState.enabled, uiState.mode, uiState.orbotStatus) {
        if (!uiState.enabled) {
            uiState =
                uiState.copy(
                    status = TorStatus.Disabled,
                    progressPercent = 0,
                    currentStep = statusDisabledText,
                    latestLogLine = logDisabledText,
                )
            appendTorLog("Tor disabled")
            return@LaunchedEffect
        }

        val awaitingValidation = !app.pendingNodeTorValidated

        when (uiState.mode) {
            TorMode.BuiltIn -> {
                syncRustTorLogs()
            }

            TorMode.External -> {
                val validationError = validateExternalConfig(uiState.externalHost, uiState.externalPort)
                uiState =
                    uiState.copy(
                        status = if (validationError == null) TorStatus.Bootstrapping else TorStatus.Error,
                        progressPercent = if (validationError == null) 50 else 0,
                        currentStep = if (validationError == null) torActionSaveConfig else torErrorConfigInvalid,
                        latestLogLine =
                            if (validationError == null) {
                                redactedProxyForLog(
                                    uiState.externalHost,
                                    uiState.externalPort.toUShortOrNull() ?: 9050u,
                                )
                            } else {
                                validationError
                            },
                    )
                appendTorLog(
                    if (validationError == null) {
                        "External Tor selected: ${
                            redactedProxyForLog(
                                uiState.externalHost,
                                uiState.externalPort.toUShortOrNull() ?: 9050u,
                            )
                        }"
                    } else {
                        "External Tor config invalid: $validationError"
                    },
                )
            }

            TorMode.Orbot -> {
                if (uiState.orbotStatus == OrbotStatus.Detected) {
                    uiState =
                        uiState.copy(
                            status = if (awaitingValidation) TorStatus.Bootstrapping else TorStatus.Ready,
                            progressPercent = if (awaitingValidation) 50 else 100,
                            currentStep = if (awaitingValidation) torActionOpenOrbot else torStatusConfigured,
                            latestLogLine = if (awaitingValidation) torActionTestConnection else torSuccessConnectionValid,
                        )
                    appendTorLog(
                        if (awaitingValidation) {
                            "Orbot mode selected, waiting for connection test"
                        } else {
                            "Orbot mode validated"
                        },
                    )
                } else {
                    uiState =
                        uiState.copy(
                            status = TorStatus.Error,
                            progressPercent = 0,
                            currentStep = torStatusInstallOrbot,
                            latestLogLine = torActionInstallOrbot,
                        )
                    appendTorLog("Orbot mode selected but Orbot is not installed")
                }
            }
        }
    }

    LaunchedEffect(
        uiState.enabled,
        uiState.mode,
        uiState.status,
        uiState.orbotStatus,
        uiState.externalHost,
        uiState.externalPort,
        app.pendingNodeAwaitingTorSetup,
        app.pendingNodeUrl,
        app.pendingNodeTorValidated,
        torTestState.running,
    ) {
        if (!app.pendingNodeAwaitingTorSetup || app.pendingNodeUrl.isBlank()) {
            autoPendingTestKey = null
            return@LaunchedEffect
        }
        if (!uiState.enabled || app.pendingNodeTorValidated || torTestState.running) {
            return@LaunchedEffect
        }

        val flowKey = "${uiState.mode}|${app.pendingNodeUrl}|${uiState.externalHost}:${uiState.externalPort}"
        if (autoPendingTestKey == flowKey) {
            return@LaunchedEffect
        }

        val readyForAutoTest =
            when (uiState.mode) {
                TorMode.BuiltIn -> uiState.status == TorStatus.Ready
                TorMode.Orbot ->
                    uiState.orbotStatus == OrbotStatus.Detected &&
                        testSocksEndpoint("127.0.0.1", 9050, 1200).isSuccess
                TorMode.External -> {
                    val externalPort = uiState.externalPort.toIntOrNull()
                    validateExternalConfig(uiState.externalHost, uiState.externalPort) == null &&
                        externalPort != null &&
                        testSocksEndpoint(uiState.externalHost, externalPort, 1200).isSuccess
                }
            }

        if (!readyForAutoTest) {
            return@LaunchedEffect
        }

        autoPendingTestKey = flowKey
        appendTorLog("Pending onion node detected; starting automatic Tor validation")
        scope.launch {
            runProgressiveTorTest()
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
                        text = stringResource(R.string.title_settings_network),
                        textAlign = TextAlign.Center,
                        modifier = Modifier.fillMaxWidth(),
                    )
                },
                navigationIcon = {
                    IconButton(onClick = { app.popRoute() }) {
                        Icon(Icons.AutoMirrored.Default.ArrowBack, contentDescription = "Back")
                    }
                },
                actions = { },
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
                SectionHeader(stringResource(R.string.title_settings_network), showDivider = false)
                MaterialSection {
                    Column {
                        networks.forEachIndexed { index, network ->
                            NetworkRow(
                                network = network,
                                isSelected = network == selectedNetwork,
                                onClick = {
                                    // show confirmation dialog before changing network
                                    pendingNetworkChange = network
                                },
                            )

                            // add divider between items, but not after the last one
                            if (index < networks.size - 1) {
                                MaterialDivider()
                            }
                        }
                    }
                }

                if (torSettingsDiscovered) {
                    SectionHeader(stringResource(R.string.tor_section_privacy), showDivider = false)
                    MaterialSection {
                        Column {
                            MaterialSettingsItem(
                                title = stringResource(R.string.tor_use_tor_title),
                                subtitle = stringResource(R.string.tor_use_tor_subtitle),
                                icon = Icons.Default.Lock,
                                isSwitch = true,
                                switchCheckedState = uiState.enabled,
                                onCheckChanged = { enabled ->
                                    Log.d(logTag, "toggle useTor: enabled=$enabled")
                                    if (!enabled) {
                                        val activeNodeIsOnion = isOnionNodeUrl(app.selectedNode.url)
                                        val pendingOnionExists =
                                            app.pendingNodeAwaitingTorSetup &&
                                                app.pendingNodeUrl.isNotBlank() &&
                                                isOnionNodeUrl(app.pendingNodeUrl)
                                        if (activeNodeIsOnion || pendingOnionExists) {
                                            showDisableTorOnionDialog = true
                                            return@MaterialSettingsItem
                                        }

                                        clearPendingOnionDraft()
                                        disableTorState()
                                        appendTorLog("Tor disabled")
                                        return@MaterialSettingsItem
                                    }

                                    if (!enableTorState()) {
                                        return@MaterialSettingsItem
                                    }
                                    uiState = uiState.copy(enabled = true)
                                    if (uiState.mode == TorMode.BuiltIn && !builtInWarmupRequested) {
                                        scope.launch {
                                            builtInWarmupRequested = true
                                            runCatching { ensureBuiltInTorBootstrap() }
                                                .onSuccess { endpoint ->
                                                    appendTorLog("Built-in Tor bootstrap started at $endpoint")
                                                }.onFailure { error ->
                                                    appendTorLog("Built-in Tor bootstrap failed: ${error.message ?: "unknown error"}")
                                                }
                                        }
                                    }
                                },
                            )
                            MaterialDivider()
                            val modeEnabled = uiState.enabled
                            val modeSubtitle = when (uiState.mode) {
                                TorMode.BuiltIn -> stringResource(R.string.tor_mode_builtin)
                                TorMode.Orbot -> stringResource(R.string.tor_mode_orbot)
                                TorMode.External -> stringResource(R.string.tor_mode_external)
                            }
                            MaterialSettingsItem(
                                title = stringResource(R.string.tor_mode_title),
                                subtitle = modeSubtitle,
                                icon = Icons.Default.Public,
                                modifier = Modifier.alpha(if (modeEnabled) 1f else 0.5f),
                                onClick = if (modeEnabled) { { showModeDialog = true } } else null,
                            )
                            MaterialDivider()
                            val statusLabel = when (uiState.status) {
                                TorStatus.Disabled -> stringResource(R.string.tor_status_disabled)
                                TorStatus.Bootstrapping -> torStatusTesting
                                TorStatus.Ready -> torStatusConfigured
                                TorStatus.Error -> torStatusActionRequired
                            }
                            MaterialSettingsItem(
                                title = stringResource(R.string.tor_status_title),
                                subtitle = statusLabel,
                                leadingContent = {
                                    Icon(
                                        imageVector = Icons.Default.Info,
                                        contentDescription = null,
                                        tint = MaterialTheme.colorScheme.primary,
                                        modifier = Modifier.size(24.dp),
                                    )
                                },
                                modifier = Modifier.alpha(if (modeEnabled) 1f else 0.5f),
                                trailingContent = {
                                    if (uiState.enabled && uiState.status == TorStatus.Ready) {
                                        Icon(
                                            imageVector = Icons.Default.Check,
                                            contentDescription = null,
                                            tint = MaterialTheme.colorScheme.primary,
                                        )
                                    }
                                },
                            )
                        }
                    }
                }

                if (torSettingsDiscovered && uiState.enabled) {
                    if (uiState.mode == TorMode.BuiltIn) {
                        SectionHeader(stringResource(R.string.tor_section_bootstrap), showDivider = false)
                        MaterialSection {
                            Column(modifier = Modifier.padding(horizontal = 16.dp, vertical = 12.dp)) {
                                Row(
                                    modifier = Modifier.fillMaxWidth(),
                                    horizontalArrangement = Arrangement.SpaceBetween,
                                    verticalAlignment = Alignment.CenterVertically,
                                ) {
                                    Text(
                                        text = stringResource(R.string.tor_progress_title),
                                        style = MaterialTheme.typography.bodyLarge,
                                    )
                                    Text(
                                        text = "${uiState.progressPercent}%",
                                        style = MaterialTheme.typography.bodyMedium,
                                        color = MaterialTheme.colorScheme.primary,
                                    )
                                }
                                LinearProgressIndicator(
                                    progress = { uiState.progressPercent / 100f },
                                    modifier =
                                        Modifier
                                            .fillMaxWidth()
                                            .padding(top = 8.dp),
                                )
                                Text(
                                    text = "${stringResource(R.string.tor_current_step_prefix)} ${uiState.currentStep}",
                                    style = MaterialTheme.typography.bodyMedium,
                                    modifier = Modifier.padding(top = 12.dp),
                                )
                                Row(
                                    modifier = Modifier.fillMaxWidth().padding(top = 12.dp),
                                    horizontalArrangement = Arrangement.spacedBy(8.dp),
                                ) {
                                    Button(
                                        onClick = {
                                            scope.launch {
                                                appendTorLog("Built-in Tor connection test requested")
                                                Log.d(logTag, "built-in test connection tapped: pendingAwaiting=${app.pendingNodeAwaitingTorSetup}, pendingValidated=${app.pendingNodeTorValidated}")
                                                runProgressiveTorTest()
                                            }
                                        },
                                        modifier = Modifier.weight(1f),
                                    ) {
                                        Text(torActionTestConnection)
                                    }
                                }
                            }
                        }

                        MaterialSection {
                            Column {
                                MaterialSettingsItem(
                                    title = stringResource(R.string.tor_view_full_log_title),
                                    subtitle = stringResource(R.string.tor_view_full_log_subtitle),
                                    icon = Icons.Default.Terminal,
                                    onClick = { showFullLogDialog = true },
                                )
                            }
                        }
                    }

                    if (uiState.mode == TorMode.External) {
                        SectionHeader(stringResource(R.string.tor_section_external), showDivider = false)
                        MaterialSection {
                            Column(modifier = Modifier.padding(horizontal = 16.dp, vertical = 12.dp)) {
                                Text(
                                    text = stringResource(R.string.tor_external_config_notice),
                                    style = MaterialTheme.typography.bodySmall,
                                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                                    modifier = Modifier.padding(bottom = 12.dp),
                                )
                                OutlinedTextField(
                                    value = uiState.externalHost,
                                    onValueChange = { value ->
                                        uiState =
                                            uiState.copy(
                                                externalHost = value,
                                                externalValidationError = validateExternalConfig(value, uiState.externalPort),
                                            )
                                    },
                                    label = { Text(stringResource(R.string.tor_external_host_label)) },
                                    singleLine = true,
                                    modifier = Modifier.fillMaxWidth(),
                                )
                                OutlinedTextField(
                                    value = uiState.externalPort,
                                    onValueChange = { value ->
                                        uiState =
                                            uiState.copy(
                                                externalPort = value,
                                                externalValidationError = validateExternalConfig(uiState.externalHost, value),
                                            )
                                    },
                                    label = { Text(stringResource(R.string.tor_external_port_label)) },
                                    singleLine = true,
                                    keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Number),
                                    isError = uiState.externalValidationError != null,
                                    supportingText = {
                                        if (uiState.externalValidationError != null) {
                                            Text(uiState.externalValidationError!!)
                                        }
                                    },
                                    modifier =
                                        Modifier
                                            .fillMaxWidth()
                                            .padding(top = 8.dp),
                                )

                                Row(
                                    modifier = Modifier.fillMaxWidth().padding(top = 12.dp),
                                    horizontalArrangement = Arrangement.spacedBy(8.dp),
                                ) {
                                    Button(
                                        onClick = {
                                            val validationError = validateExternalConfig(uiState.externalHost, uiState.externalPort)
                                            if (validationError != null) {
                                                uiState = uiState.copy(externalValidationError = validationError)
                                                scope.launch { snackbarHostState.showSnackbar(validationError) }
                                                return@Button
                                            }

                                            val proxyLog =
                                                redactedProxyForLog(
                                                    uiState.externalHost,
                                                    uiState.externalPort.toUShortOrNull() ?: 9050u,
                                                )
                                            Log.d(logTag, "save external tor config: proxy=$proxyLog")
                                            val externalPort = uiState.externalPort.toUShortOrNull()
                                            if (externalPort == null ||
                                                !saveExternalTorConfig(uiState.externalHost, externalPort, proxyLog)
                                            ) {
                                                return@Button
                                            }
                                            app.pendingNodeTorValidated = false
                                            appendTorLog("Saved external Tor config: $proxyLog")
                                            scope.launch { snackbarHostState.showSnackbar(torSuccessConfigurationSaved) }
                                        },
                                        modifier = Modifier.weight(1f),
                                    ) {
                                        Text(torActionSaveConfig)
                                    }

                                    Button(
                                        onClick = {
                                            val validationError = validateExternalConfig(uiState.externalHost, uiState.externalPort)
                                            if (validationError != null) {
                                                uiState = uiState.copy(externalValidationError = validationError)
                                                scope.launch { snackbarHostState.showSnackbar(validationError) }
                                                return@Button
                                            }

                                            scope.launch {
                                                val proxyLog =
                                                    redactedProxyForLog(
                                                        uiState.externalHost,
                                                        uiState.externalPort.toUShortOrNull() ?: 9050u,
                                                    )
                                                appendTorLog("External Tor connection test requested for $proxyLog")
                                                Log.d(logTag, "external test connection tapped: proxy=$proxyLog, pendingAwaiting=${app.pendingNodeAwaitingTorSetup}, pendingValidated=${app.pendingNodeTorValidated}")
                                                runProgressiveTorTest()
                                            }
                                        },
                                        modifier = Modifier.weight(1f),
                                    ) {
                                        Text(torActionTestConnection)
                                    }
                                }
                            }
                        }
                    }

                    if (uiState.mode == TorMode.Orbot) {
                        SectionHeader(stringResource(R.string.tor_orbot_status_title), showDivider = false)
                        MaterialSection {
                            Column {
                                val orbotTitle = when (uiState.orbotStatus) {
                                    OrbotStatus.Checking -> stringResource(R.string.tor_orbot_checking)
                                    OrbotStatus.Detected -> {
                                        val version = uiState.orbotVersion
                                        if (version.isNullOrBlank()) {
                                            stringResource(R.string.tor_orbot_detected)
                                        } else {
                                            stringResource(R.string.tor_orbot_detected_version, version)
                                        }
                                    }
                                    OrbotStatus.NotDetected -> stringResource(R.string.tor_orbot_not_detected)
                                }
                                val orbotSubtitle = if (uiState.orbotStatus == OrbotStatus.Detected) {
                                    stringResource(R.string.tor_open_orbot_subtitle)
                                } else {
                                    stringResource(R.string.tor_install_orbot_subtitle)
                                }
                                MaterialSettingsItem(
                                    title = orbotTitle,
                                    subtitle = orbotSubtitle,
                                    icon = Icons.Default.Settings,
                                    onClick = {
                                        if (uiState.orbotStatus == OrbotStatus.Detected) {
                                            OrbotPackageHelper.openOrbot(context)
                                        } else {
                                            OrbotPackageHelper.openInstallPage(context)
                                        }
                                    },
                                )
                                MaterialDivider()
                                MaterialSettingsItem(
                                    title = stringResource(R.string.tor_detect_orbot_title),
                                    subtitle = stringResource(R.string.tor_detect_orbot_subtitle),
                                    icon = Icons.Default.Refresh,
                                    onClick = {
                                        val (status, version) = OrbotPackageHelper.detect(context)
                                        uiState = uiState.copy(orbotStatus = status, orbotVersion = version)
                                        appendTorLog(
                                            when (status) {
                                                OrbotStatus.Detected -> "Detected Orbot${version?.let { " ($it)" } ?: ""}"
                                                OrbotStatus.NotDetected -> "Orbot not detected"
                                                OrbotStatus.Checking -> "Checking Orbot status"
                                            },
                                        )
                                    },
                                )

                                Row(
                                    modifier = Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 12.dp),
                                    horizontalArrangement = Arrangement.spacedBy(8.dp),
                                ) {
                                    Button(
                                        onClick = {
                                            if (uiState.orbotStatus == OrbotStatus.Detected) {
                                                OrbotPackageHelper.openOrbot(context)
                                            } else {
                                                OrbotPackageHelper.openInstallPage(context)
                                            }
                                        },
                                        modifier = Modifier.weight(1f),
                                    ) {
                                        Text(if (uiState.orbotStatus == OrbotStatus.Detected) torActionOpenOrbot else torActionInstallOrbot)
                                    }

                                    Button(
                                        onClick = {
                                            if (uiState.orbotStatus != OrbotStatus.Detected) {
                                                scope.launch { snackbarHostState.showSnackbar(torStatusInstallOrbot) }
                                                return@Button
                                            }

                                            scope.launch {
                                                appendTorLog("Orbot Tor connection test requested")
                                                Log.d(logTag, "orbot test connection tapped: pendingAwaiting=${app.pendingNodeAwaitingTorSetup}, pendingValidated=${app.pendingNodeTorValidated}")
                                                runProgressiveTorTest()
                                            }
                                        },
                                        modifier = Modifier.weight(1f),
                                    ) {
                                        Text(torActionTestConnection)
                                    }
                                }
                            }
                        }
                    }
                }
            }
        },
    )

    if (showModeDialog) {
        AlertDialog(
            onDismissRequest = { showModeDialog = false },
            title = { Text(stringResource(R.string.tor_mode_title)) },
            text = {
                Column {
                    TorModeOption(
                        title = stringResource(R.string.tor_mode_builtin),
                        selected = uiState.mode == TorMode.BuiltIn,
                        onClick = {
                            Log.d(logTag, "set tor mode: BUILT_IN")
                            if (!setPersistedTorMode(TorMode.BuiltIn)) {
                                return@TorModeOption
                            }
                            app.pendingNodeTorValidated = false
                            autoPendingTestKey = null
                            uiState = uiState.copy(mode = TorMode.BuiltIn)
                            appendTorLog("Switched Tor mode to built-in")
                            builtInWarmupRequested = false
                            if (uiState.enabled) {
                                scope.launch {
                                    builtInWarmupRequested = true
                                    runCatching { ensureBuiltInTorBootstrap() }
                                        .onSuccess { endpoint ->
                                            appendTorLog("Built-in Tor bootstrap started at $endpoint")
                                        }.onFailure { error ->
                                            appendTorLog("Built-in Tor bootstrap failed: ${error.message ?: "unknown error"}")
                                        }
                                }
                            }
                            showModeDialog = false
                        },
                    )
                    TorModeOption(
                        title = stringResource(R.string.tor_mode_orbot),
                        selected = uiState.mode == TorMode.Orbot,
                        onClick = {
                            Log.d(logTag, "set tor mode: ORBOT")
                            if (!setPersistedTorMode(TorMode.Orbot)) {
                                return@TorModeOption
                            }
                            app.pendingNodeTorValidated = false
                            autoPendingTestKey = null
                            uiState = uiState.copy(mode = TorMode.Orbot)
                            appendTorLog("Switched Tor mode to Orbot")
                            showModeDialog = false
                        },
                    )
                    TorModeOption(
                        title = stringResource(R.string.tor_mode_external),
                        selected = uiState.mode == TorMode.External,
                        onClick = {
                            Log.d(logTag, "set tor mode: EXTERNAL")
                            if (!setPersistedTorMode(TorMode.External)) {
                                return@TorModeOption
                            }
                            app.pendingNodeTorValidated = false
                            autoPendingTestKey = null
                            uiState = uiState.copy(mode = TorMode.External)
                            appendTorLog("Switched Tor mode to external proxy")
                            showModeDialog = false
                        },
                    )
                }
            },
            confirmButton = {
                TextButton(onClick = { showModeDialog = false }) {
                    Text(stringResource(R.string.btn_cancel))
                }
            },
        )
    }

    if (showDisableTorOnionDialog) {
        AlertDialog(
            onDismissRequest = { showDisableTorOnionDialog = false },
            title = { Text(torDisableOnionDialogTitle) },
            text = { Text(torDisableOnionDialogBody) },
            confirmButton = {
                TextButton(
                    onClick = {
                        showDisableTorOnionDialog = false
                        scope.launch {
                            if (isOnionNodeUrl(app.selectedNode.url)) {
                                val fallbackResult = switchToFirstClearnetPresetNode(nodeSelector)
                                if (fallbackResult.isFailure) {
                                    val reason =
                                        fallbackResult.exceptionOrNull()?.message
                                            ?: "unknown error"
                                    appendTorLog("Unable to disable Tor: clearnet fallback failed ($reason)")
                                    snackbarHostState.showSnackbar("$torFallbackFailed: $reason")
                                    return@launch
                                }

                                val fallbackNode = fallbackResult.getOrThrow()
                                appendTorLog(
                                    "Switched active node to ${redactedNodeForLog(fallbackNode)} before disabling Tor",
                                )
                                snackbarHostState.showSnackbar(
                                    torFallbackApplied.format(fallbackNode.name),
                                )
                            } else if (
                                app.pendingNodeAwaitingTorSetup &&
                                app.pendingNodeUrl.isNotBlank() &&
                                isOnionNodeUrl(app.pendingNodeUrl)
                            ) {
                                appendTorLog("Discarded pending onion node draft before disabling Tor")
                            }

                            clearPendingOnionDraft()
                            disableTorState()
                            appendTorLog("Tor disabled")
                        }
                    },
                ) {
                    Text(torDisableOnionDialogConfirm)
                }
            },
            dismissButton = {
                TextButton(onClick = { showDisableTorOnionDialog = false }) {
                    Text(torDisableOnionDialogCancel)
                }
            },
        )
    }

    // network change confirmation dialog
    pendingNetworkChange?.let { network ->
        AlertDialog(
            onDismissRequest = { pendingNetworkChange = null },
            title = { Text("Warning: Network Changed") },
            text = { Text("You've changed your network to ${network.toString()}") },
            confirmButton = {
                TextButton(
                    onClick = {
                        pendingNetworkChange = null
                        app.dispatch(AppAction.ChangeNetwork(network))
                        app.trySelectLatestOrNewWallet()
                        app.popRoute()
                    },
                ) {
                    Text("Yes, Change Network")
                }
            },
            dismissButton = {
                TextButton(
                    onClick = { pendingNetworkChange = null },
                ) {
                    Text("Cancel")
                }
            },
        )
    }

    if (showFullLogDialog) {
        val fullLogText = uiState.logLines.joinToString(separator = "\n")
        AlertDialog(
            onDismissRequest = { showFullLogDialog = false },
            title = { Text(stringResource(R.string.tor_full_log_title)) },
            text = {
                Column(
                    modifier =
                        Modifier
                            .verticalScroll(rememberScrollState())
                            .background(MaterialTheme.colorScheme.surfaceVariant, MaterialTheme.shapes.small)
                            .padding(8.dp),
                ) {
                    uiState.logLines.forEach { line ->
                        Text(
                            text = "> $line",
                            style = MaterialTheme.typography.labelSmall,
                            fontFamily = androidx.compose.ui.text.font.FontFamily.Monospace,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                            modifier = Modifier.padding(bottom = 2.dp),
                        )
                    }
                }
            },
            confirmButton = {
                TextButton(onClick = { showFullLogDialog = false }) {
                    Text(stringResource(R.string.btn_done))
                }
            },
            dismissButton = {
                TextButton(
                    onClick = {
                        clipboardManager.setText(AnnotatedString(fullLogText))
                        scope.launch { snackbarHostState.showSnackbar(copiedText) }
                    },
                ) {
                    Text(stringResource(R.string.btn_copy))
                }
            },
        )
    }

    if (showTorTestDialog) {
        val sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true)
        ModalBottomSheet(
            onDismissRequest = {
                if (!torTestState.running) {
                    showTorTestDialog = false
                }
            },
            sheetState = sheetState,
            containerColor = MaterialTheme.colorScheme.surface,
        ) {
            TorTestSheetContent(
                state = torTestState,
                onDismiss = { showTorTestDialog = false }
            )
        }
    }
}

@Composable
private fun TorTestSheetContent(
    state: TorConnectionTestState,
    onDismiss: () -> Unit,
) {
    Column(
        modifier =
            Modifier
                .fillMaxWidth()
                .padding(horizontal = 24.dp)
                .padding(bottom = 32.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        Text(
            text = "Connection Test",
            style = MaterialTheme.typography.headlineSmall,
            fontWeight = FontWeight.Bold,
            modifier = Modifier.padding(vertical = 16.dp)
        )

        Column(
            modifier = Modifier.fillMaxWidth(),
            verticalArrangement = Arrangement.spacedBy(16.dp)
        ) {
            state.steps.forEach { step ->
                TorTestStepProgressRow(step = step)
            }
        }

        Spacer(modifier = Modifier.height(32.dp))

        Column(
            modifier =
                Modifier
                    .fillMaxWidth()
                    .clip(RoundedCornerShape(12.dp))
                    .background(CoveColor.midnightBlue)
                    .padding(16.dp)
        ) {
            Row(
                modifier = Modifier.fillMaxWidth(),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(8.dp)
            ) {
                Icon(
                    imageVector = Icons.Default.Terminal,
                    contentDescription = null,
                    tint = Color.White.copy(alpha = 0.7f),
                    modifier = Modifier.size(14.dp)
                )
                Text(
                    text = "LIVE TEST LOGS",
                    style = MaterialTheme.typography.labelSmall,
                    fontWeight = FontWeight.Bold,
                    color = Color.White.copy(alpha = 0.7f),
                    letterSpacing = 0.5.sp
                )
            }

            Spacer(modifier = Modifier.height(12.dp))

            val scrollState = rememberScrollState()
            LaunchedEffect(state.logs.size) {
                scrollState.animateScrollTo(scrollState.maxValue)
            }

            Column(
                modifier =
                    Modifier
                        .fillMaxWidth()
                        .height(120.dp)
                        .verticalScroll(scrollState)
            ) {
                state.logs.forEach { line ->
                    Text(
                        text = line,
                        style = MaterialTheme.typography.labelSmall,
                        fontFamily = androidx.compose.ui.text.font.FontFamily.Monospace,
                        color = Color.White.copy(alpha = 0.9f),
                        modifier = Modifier.padding(bottom = 2.dp)
                    )
                }
            }
        }

        Spacer(modifier = Modifier.height(32.dp))

        Button(
            onClick = onDismiss,
            enabled = !state.running,
            modifier =
                Modifier
                    .fillMaxWidth()
                    .height(50.dp),
            shape = RoundedCornerShape(12.dp)
        ) {
            Text(
                text = if (state.running) "Running Test..." else "Done",
                fontWeight = FontWeight.SemiBold
            )
        }
    }
}

@Composable
private fun TorTestStepProgressRow(step: TorTestStep) {
    val statusColor = when (step.status) {
        TorTestStepStatus.Passed -> Color(0xFF34C759)
        TorTestStepStatus.Failed -> Color(0xFFFF3B30)
        TorTestStepStatus.Running -> MaterialTheme.colorScheme.primary
        TorTestStepStatus.Pending -> MaterialTheme.colorScheme.outline.copy(alpha = 0.4f)
    }

    Row(
        modifier = Modifier.fillMaxWidth(),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(16.dp)
    ) {
        Box(
            modifier = Modifier.size(24.dp),
            contentAlignment = Alignment.Center
        ) {
            when (step.status) {
                TorTestStepStatus.Pending -> {
                    androidx.compose.foundation.Canvas(modifier = Modifier.size(12.dp)) {
                        drawCircle(color = statusColor, style = androidx.compose.ui.graphics.drawscope.Stroke(width = 2.dp.toPx()))
                    }
                }
                TorTestStepStatus.Running -> {
                    CircularProgressIndicator(
                        modifier = Modifier.size(20.dp),
                        strokeWidth = 2.dp,
                        color = statusColor
                    )
                }
                TorTestStepStatus.Passed -> {
                    Icon(
                        imageVector = Icons.Default.CheckCircle,
                        contentDescription = null,
                        tint = statusColor,
                        modifier = Modifier.size(24.dp)
                    )
                }
                TorTestStepStatus.Failed -> {
                    Icon(
                        imageVector = Icons.Default.Close,
                        contentDescription = null,
                        tint = statusColor,
                        modifier = Modifier.size(24.dp)
                    )
                }
            }
        }

        Column(modifier = Modifier.weight(1f)) {
            Text(
                text = step.title,
                style = MaterialTheme.typography.bodyLarge,
                fontWeight = FontWeight.SemiBold,
                color = if (step.status == TorTestStepStatus.Pending) MaterialTheme.colorScheme.onSurface.copy(alpha = 0.4f) else MaterialTheme.colorScheme.onSurface
            )
            if (step.status == TorTestStepStatus.Running || step.status == TorTestStepStatus.Failed || step.status == TorTestStepStatus.Passed) {
                Text(
                    text = step.detail,
                    style = MaterialTheme.typography.labelMedium,
                    color = MaterialTheme.colorScheme.onSurfaceVariant
                )
            }
        }
    }
}

private fun parseTorMode(mode: String?): TorMode {
    return when (parseCoreTorMode(mode)) {
        CoreTorMode.ORBOT -> TorMode.Orbot
        CoreTorMode.EXTERNAL -> TorMode.External
        CoreTorMode.BUILT_IN -> TorMode.BuiltIn
    }
}

private fun validateExternalConfig(host: String, port: String): String? {
    if (host.isBlank()) return "Host is required"
    val parsed = port.toIntOrNull() ?: return "Port must be a number"
    if (parsed !in 1..65535) return "Port must be between 1 and 65535"
    return null
}

@Composable
private fun NetworkRow(
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
        horizontalArrangement = Arrangement.SpaceBetween,
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Text(
            text = network.toString(),
            style = MaterialTheme.typography.bodyLarge,
        )

        if (isSelected) {
            Icon(
                imageVector = Icons.Default.Check,
                contentDescription = "Selected",
                tint = MaterialTheme.colorScheme.primary,
            )
        }
    }
}

@Composable
private fun TorModeOption(
    title: String,
    selected: Boolean,
    onClick: () -> Unit,
) {
    Row(
        modifier =
            Modifier
                .fillMaxWidth()
                .clickable(onClick = onClick)
                .padding(vertical = 8.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        RadioButton(selected = selected, onClick = onClick)
        Text(
            text = title,
            style = MaterialTheme.typography.bodyLarge,
            modifier = Modifier.padding(start = 8.dp),
        )
    }
}
