@file:Suppress("FunctionNaming", "PackageNaming")

package org.bitcoinppl.cove.flows.SettingsFlow

import android.content.Context
import androidx.compose.foundation.background
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
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.outlined.Help
import androidx.compose.material.icons.filled.Add
import androidx.compose.material.icons.filled.Check
import androidx.compose.material.icons.filled.Delete
import androidx.compose.material.icons.filled.Dns
import androidx.compose.material.icons.filled.Hub
import androidx.compose.material3.Button
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.ExtendedFloatingActionButton
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Scaffold
import androidx.compose.material3.SnackbarDuration
import androidx.compose.material3.SnackbarHost
import androidx.compose.material3.SnackbarHostState
import androidx.compose.material3.SnackbarResult
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.rememberModalBottomSheetState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalSoftwareKeyboardController
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.input.KeyboardCapitalization
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.launch
import org.bitcoinppl.cove.R
import org.bitcoinppl.cove.TaggedItem
import org.bitcoinppl.cove.ui.theme.coveColors
import org.bitcoinppl.cove_core.AppAlertState
import org.bitcoinppl.cove_core.Database
import org.bitcoinppl.cove_core.DatabaseException
import org.bitcoinppl.cove_core.GlobalConfigTableException

private val DEFAULT_RELAYS =
    listOf(
        "https://relay.payjoin.org",
        "https://ohttp.achow101.com",
        "https://pj.bobspacebkk.com",
    )

private const val PREFS_NAME = "ohttp_relay_settings"
private const val KEY_BACKED_UP_RELAYS = "backed_up_custom_relays"

private fun displayDomain(url: String): String =
    url.removePrefix("https://").removePrefix("http://").trimEnd('/')

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun OhttpRelaySettingsScreen(
    app: org.bitcoinppl.cove.AppManager,
    modifier: Modifier = Modifier,
) {
    val config = remember { Database().globalConfig() }
    val snackbarHostState = remember { SnackbarHostState() }
    val scope = rememberCoroutineScope()
    val keyboardController = LocalSoftwareKeyboardController.current
    val context = LocalContext.current
    val prefs = remember { context.getSharedPreferences(PREFS_NAME, Context.MODE_PRIVATE) }
    val invalidUrlTitle = stringResource(R.string.ohttp_relay_invalid_url_title)
    val invalidUrlMessage = stringResource(R.string.ohttp_relay_invalid_url_message)
    val updateFailedTitle = stringResource(R.string.ohttp_relay_update_failed_title)
    val updateFailedMessage = stringResource(R.string.ohttp_relay_update_failed_message)
    val relayRemovedMessage = stringResource(R.string.ohttp_relay_removed)
    val undoLabel = stringResource(R.string.ohttp_relay_undo)

    val initialRelays = remember { config.ohttpRelayUrls() }
    val initialBackup = remember {
        prefs
            .getString(KEY_BACKED_UP_RELAYS, null)
            ?.split("\n")
            ?.filter { it.isNotEmpty() }
            .orEmpty()
    }

    var relays by remember { mutableStateOf(initialRelays) }
    var backedUpRelays by remember {
        if (initialRelays.isNotEmpty() && initialBackup.isNotEmpty()) {
            mutableStateOf(emptyList())
        } else {
            mutableStateOf(initialBackup)
        }
    }

    LaunchedEffect(Unit) {
        if (initialRelays.isNotEmpty() && initialBackup.isNotEmpty()) {
            prefs.edit().remove(KEY_BACKED_UP_RELAYS).apply()
        }
    }

    var showHowItWorksSheet by remember { mutableStateOf(false) }
    var showAddRelaySheet by remember { mutableStateOf(false) }

    val customActive = relays.isNotEmpty()
    val defaultsActive = relays.isEmpty()

    fun showAlert(
        title: String,
        message: String,
    ) {
        app.alertState =
            TaggedItem(
                AppAlertState.General(
                    title = title,
                    message = message,
                ),
            )
    }

    fun save(newRelays: List<String>): Boolean {
        return try {
            relays = config.setOhttpRelayUrls(newRelays)
            true
        } catch (e: Exception) {
            if (e is DatabaseException.GlobalConfig &&
                e.v1 is GlobalConfigTableException.InvalidOhttpRelayUrl
            ) {
                showAlert(invalidUrlTitle, invalidUrlMessage)
            } else {
                showAlert(updateFailedTitle, updateFailedMessage)
            }
            false
        }
    }

    fun useDefaultsInstead() {
        if (relays.isEmpty()) return
        val currentRelays = relays.toList()
        if (save(emptyList())) {
            prefs.edit().putString(KEY_BACKED_UP_RELAYS, currentRelays.joinToString("\n")).apply()
            backedUpRelays = currentRelays
        }
    }

    fun useCustomInstead() {
        if (backedUpRelays.isEmpty()) return
        if (save(backedUpRelays)) {
            prefs.edit().remove(KEY_BACKED_UP_RELAYS).apply()
            backedUpRelays = emptyList()
        }
    }

    fun deleteBackedUpRelay(index: Int) {
        if (index !in backedUpRelays.indices) return
        val updated = backedUpRelays.toMutableList().also { it.removeAt(index) }
        backedUpRelays = updated
        if (updated.isEmpty()) {
            prefs.edit().remove(KEY_BACKED_UP_RELAYS).apply()
        } else {
            prefs.edit().putString(KEY_BACKED_UP_RELAYS, updated.joinToString("\n")).apply()
        }
    }

    fun deleteRelay(index: Int) {
        if (index !in relays.indices) return
        val deleted = relays[index]
        val updated = relays.toMutableList().also { it.removeAt(index) }
        if (save(updated)) {
            scope.launch {
                val result =
                    snackbarHostState.showSnackbar(
                        message = relayRemovedMessage,
                        actionLabel = undoLabel,
                        duration = SnackbarDuration.Short,
                    )
                if (result == SnackbarResult.ActionPerformed) {
                    val restored = relays.toMutableList()
                    restored.add(minOf(index, restored.size), deleted)
                    save(restored)
                }
            }
        }
    }

    fun addRelay(url: String): Boolean {
        val trimmed = url.trim()
        if (trimmed.isEmpty()) return false

        val baseList =
            if (defaultsActive && backedUpRelays.isNotEmpty()) {
                backedUpRelays
            } else {
                relays
            }

        val success = save(baseList + trimmed)
        if (success && backedUpRelays.isNotEmpty()) {
            prefs.edit().remove(KEY_BACKED_UP_RELAYS).apply()
            backedUpRelays = emptyList()
        }
        return success
    }

    Scaffold(
        modifier =
            modifier
                .fillMaxSize()
                .padding(WindowInsets.safeDrawing.asPaddingValues()),
        snackbarHost = { SnackbarHost(snackbarHostState) },
        topBar = {
            SettingsTopAppBar(
                title = stringResource(R.string.title_settings_ohttp_relay),
                onBack = { app.popRoute() },
                actions = {
                    IconButton(onClick = { showHowItWorksSheet = true }) {
                        Icon(
                            imageVector = Icons.AutoMirrored.Outlined.Help,
                            contentDescription = stringResource(R.string.ohttp_relay_how_it_works_title),
                        )
                    }
                },
            )
        },
        floatingActionButton = {
            ExtendedFloatingActionButton(
                onClick = { showAddRelaySheet = true },
                icon = {
                    Icon(
                        imageVector = Icons.Default.Add,
                        contentDescription = null,
                    )
                },
                text = { Text(stringResource(R.string.ohttp_relay_add_relay)) },
                containerColor = MaterialTheme.colorScheme.primary,
                contentColor = MaterialTheme.colorScheme.onPrimary,
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
            Text(
                text = stringResource(R.string.ohttp_relay_description),
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                modifier = Modifier.padding(horizontal = 16.dp, vertical = 8.dp),
            )

            Spacer(modifier = Modifier.height(8.dp))

            RelaysSectionHeader(
                title = stringResource(R.string.ohttp_relay_default_relays),
                inUse = defaultsActive,
                trailingAction =
                    if (customActive) {
                        {
                            SwitchModeButton(
                                text = stringResource(R.string.ohttp_relay_use_defaults_instead),
                                onClick = ::useDefaultsInstead,
                            )
                        }
                    } else {
                        null
                    },
            )
            Column(
                modifier = Modifier.padding(horizontal = 16.dp),
                verticalArrangement = Arrangement.spacedBy(6.dp),
            ) {
                val textColor =
                    if (customActive) {
                        MaterialTheme.colorScheme.onSurfaceVariant.copy(alpha = 0.5f)
                    } else {
                        MaterialTheme.colorScheme.onSurface
                    }
                val iconColor =
                    if (customActive) {
                        MaterialTheme.colorScheme.onSurfaceVariant.copy(alpha = 0.4f)
                    } else {
                        MaterialTheme.colorScheme.onSurfaceVariant
                    }
                DEFAULT_RELAYS.forEach { relay ->
                    RelayCard(
                        domain = displayDomain(relay),
                        backgroundColor = MaterialTheme.colorScheme.surfaceVariant,
                        textColor = textColor,
                        iconColor = iconColor,
                    )
                }
            }

            Spacer(modifier = Modifier.height(16.dp))

            RelaysSectionHeader(
                title = stringResource(R.string.ohttp_relay_custom_section),
                inUse = customActive,
                trailingAction =
                    if (defaultsActive && backedUpRelays.isNotEmpty()) {
                        {
                            SwitchModeButton(
                                text = stringResource(R.string.ohttp_relay_use_custom_instead),
                                onClick = ::useCustomInstead,
                            )
                        }
                    } else {
                        null
                    },
            )

            if (customActive) {
                Column(
                    modifier = Modifier.padding(horizontal = 16.dp),
                    verticalArrangement = Arrangement.spacedBy(6.dp),
                ) {
                    relays.forEachIndexed { index, relay ->
                        RelayCard(
                            domain = displayDomain(relay),
                            backgroundColor = MaterialTheme.colorScheme.primaryContainer,
                            textColor = MaterialTheme.colorScheme.onPrimaryContainer,
                            iconColor = MaterialTheme.colorScheme.onPrimaryContainer,
                            trailingContent = {
                                DeleteButton(onClick = { deleteRelay(index) })
                            },
                        )
                    }
                }
            } else if (backedUpRelays.isNotEmpty()) {
                Column(
                    modifier = Modifier.padding(horizontal = 16.dp),
                    verticalArrangement = Arrangement.spacedBy(6.dp),
                ) {
                    val mutedColor = MaterialTheme.colorScheme.onSurfaceVariant.copy(alpha = 0.5f)
                    backedUpRelays.forEachIndexed { index, relay ->
                        RelayCard(
                            domain = displayDomain(relay),
                            backgroundColor = MaterialTheme.colorScheme.surfaceVariant,
                            textColor = mutedColor,
                            iconColor = mutedColor,
                            trailingContent = {
                                DeleteButton(onClick = { deleteBackedUpRelay(index) }, muted = true)
                            },
                        )
                    }
                }
            } else {
                EmptyRelaysCard()
            }

            Text(
                text =
                    if (customActive) {
                        stringResource(R.string.ohttp_relay_footer_custom)
                    } else {
                        stringResource(R.string.ohttp_relay_footer_defaults)
                    },
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                modifier = Modifier.padding(horizontal = 16.dp, vertical = 8.dp),
            )

            Spacer(modifier = Modifier.height(72.dp))
        }
    }

    if (showHowItWorksSheet) {
        HowRelaysWorkSheet(onDismiss = { showHowItWorksSheet = false })
    }

    if (showAddRelaySheet) {
        AddRelaySheet(
            onAdd = { url ->
                if (addRelay(url)) {
                    keyboardController?.hide()
                    showAddRelaySheet = false
                }
            },
            onDismiss = {
                keyboardController?.hide()
                showAddRelaySheet = false
            },
        )
    }
}

@Composable
private fun RelaysSectionHeader(
    title: String,
    modifier: Modifier = Modifier,
    inUse: Boolean = false,
    trailingAction: @Composable (() -> Unit)? = null,
) {
    Row(
        modifier =
            modifier
                .fillMaxWidth()
                .padding(start = 16.dp, end = 8.dp, top = 4.dp, bottom = 8.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Text(
            text = title,
            style = MaterialTheme.typography.labelMedium,
            color = MaterialTheme.colorScheme.primary,
            modifier = Modifier.weight(1f),
        )
        if (inUse) {
            InUseBadge()
            Spacer(modifier = Modifier.width(4.dp))
        }
        trailingAction?.invoke()
    }
}

@Composable
private fun InUseBadge() {
    Row(
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(4.dp),
    ) {
        Icon(
            imageVector = Icons.Default.Check,
            contentDescription = null,
            tint = MaterialTheme.coveColors.systemGreen,
            modifier = Modifier.size(16.dp),
        )
        Text(
            text = stringResource(R.string.ohttp_relay_in_use),
            style = MaterialTheme.typography.labelMedium,
            color = MaterialTheme.coveColors.systemGreen,
        )
    }
}

@Composable
private fun SwitchModeButton(
    text: String,
    onClick: () -> Unit,
) {
    OutlinedButton(
        onClick = onClick,
        contentPadding = PaddingValues(horizontal = 12.dp, vertical = 6.dp),
    ) {
        Text(
            text = text,
            style = MaterialTheme.typography.labelSmall,
        )
    }
}

@Composable
private fun RelayCard(
    domain: String,
    backgroundColor: Color,
    textColor: Color,
    iconColor: Color,
    modifier: Modifier = Modifier,
    trailingContent: @Composable (() -> Unit)? = null,
) {
    Surface(
        modifier = modifier.fillMaxWidth(),
        shape = MaterialTheme.shapes.medium,
        color = backgroundColor,
    ) {
        Row(
            modifier = Modifier.padding(horizontal = 16.dp, vertical = 14.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Icon(
                imageVector = Icons.Default.Dns,
                contentDescription = null,
                tint = iconColor,
                modifier = Modifier.size(22.dp),
            )
            Spacer(modifier = Modifier.width(12.dp))
            Text(
                text = domain,
                style = MaterialTheme.typography.bodyMedium,
                color = textColor,
                modifier = Modifier.weight(1f),
            )
            trailingContent?.invoke()
        }
    }
}

@Composable
private fun DeleteButton(
    onClick: () -> Unit,
    muted: Boolean = false,
) {
    val tint =
        if (muted) {
            MaterialTheme.colorScheme.onSurfaceVariant.copy(alpha = 0.35f)
        } else {
            MaterialTheme.colorScheme.onSurfaceVariant
        }
    val bgColor =
        if (muted) {
            MaterialTheme.colorScheme.surface.copy(alpha = 0.5f)
        } else {
            MaterialTheme.colorScheme.surface
        }
    IconButton(onClick = onClick) {
        Box(
            modifier =
                Modifier
                    .size(36.dp)
                    .background(bgColor, CircleShape),
            contentAlignment = Alignment.Center,
        ) {
            Icon(
                imageVector = Icons.Default.Delete,
                contentDescription = null,
                tint = tint,
                modifier = Modifier.size(18.dp),
            )
        }
    }
}

@Composable
private fun EmptyRelaysCard() {
    Surface(
        modifier =
            Modifier
                .fillMaxWidth()
                .padding(horizontal = 16.dp, vertical = 8.dp),
        shape = MaterialTheme.shapes.medium,
        color = MaterialTheme.colorScheme.surfaceVariant,
    ) {
        Column(
            modifier = Modifier.padding(24.dp),
            horizontalAlignment = Alignment.CenterHorizontally,
            verticalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            Icon(
                imageVector = Icons.Default.Hub,
                contentDescription = null,
                tint = MaterialTheme.colorScheme.onSurfaceVariant,
                modifier = Modifier.size(36.dp),
            )
            Text(
                text = stringResource(R.string.ohttp_relay_empty_title),
                style = MaterialTheme.typography.titleSmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                fontWeight = FontWeight.SemiBold,
            )
            Text(
                text = stringResource(R.string.ohttp_relay_empty_subtitle),
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                textAlign = TextAlign.Center,
            )
        }
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun HowRelaysWorkSheet(onDismiss: () -> Unit) {
    val sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true)

    ModalBottomSheet(
        onDismissRequest = onDismiss,
        sheetState = sheetState,
        containerColor = MaterialTheme.colorScheme.surface,
    ) {
        Column(
            modifier =
                Modifier
                    .fillMaxWidth()
                    .padding(horizontal = 24.dp)
                    .padding(bottom = 32.dp),
            verticalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            Text(
                text = stringResource(R.string.ohttp_relay_how_it_works_title),
                style = MaterialTheme.typography.titleLarge,
                fontWeight = FontWeight.Bold,
            )
            Text(
                text = stringResource(R.string.ohttp_relay_how_it_works_body),
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
            Spacer(modifier = Modifier.height(4.dp))
            Button(
                onClick = onDismiss,
                modifier = Modifier.fillMaxWidth(),
            ) {
                Text(stringResource(R.string.ohttp_relay_got_it))
            }
        }
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun AddRelaySheet(
    onAdd: (String) -> Unit,
    onDismiss: () -> Unit,
) {
    val sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true)
    var input by remember { mutableStateOf("") }

    ModalBottomSheet(
        onDismissRequest = onDismiss,
        sheetState = sheetState,
        containerColor = MaterialTheme.colorScheme.surface,
    ) {
        Column(
            modifier =
                Modifier
                    .fillMaxWidth()
                    .padding(horizontal = 24.dp)
                    .padding(bottom = 32.dp),
            verticalArrangement = Arrangement.spacedBy(16.dp),
        ) {
            Text(
                text = stringResource(R.string.ohttp_relay_add_relay),
                style = MaterialTheme.typography.titleLarge,
                fontWeight = FontWeight.Bold,
            )
            OutlinedTextField(
                value = input,
                onValueChange = { input = it },
                label = { Text(stringResource(R.string.ohttp_relay_url_placeholder)) },
                keyboardOptions =
                    KeyboardOptions(
                        capitalization = KeyboardCapitalization.None,
                        imeAction = ImeAction.Done,
                        keyboardType = KeyboardType.Uri,
                    ),
                keyboardActions =
                    KeyboardActions(
                        onDone = { if (input.trim().isNotEmpty()) onAdd(input) },
                    ),
                singleLine = true,
                modifier = Modifier.fillMaxWidth(),
            )
            Row(
                modifier = Modifier.fillMaxWidth(),
                horizontalArrangement = Arrangement.spacedBy(8.dp, Alignment.End),
            ) {
                TextButton(onClick = onDismiss) {
                    Text(stringResource(R.string.btn_cancel))
                }
                Button(
                    onClick = { onAdd(input) },
                    enabled = input.trim().isNotEmpty(),
                ) {
                    Text(stringResource(R.string.ohttp_relay_add))
                }
            }
        }
    }
}
