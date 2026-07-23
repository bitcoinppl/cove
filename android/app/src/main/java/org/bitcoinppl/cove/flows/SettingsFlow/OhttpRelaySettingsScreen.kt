package org.bitcoinppl.cove.flows.SettingsFlow

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.asPaddingValues
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.safeDrawing
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Delete
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Scaffold
import androidx.compose.material3.SnackbarHost
import androidx.compose.material3.SnackbarHostState
import androidx.compose.material3.SwipeToDismissBox
import androidx.compose.material3.SwipeToDismissBoxValue
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.rememberSwipeToDismissBoxState
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
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.input.KeyboardCapitalization
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.launch
import org.bitcoinppl.cove.R
import org.bitcoinppl.cove.TaggedItem
import org.bitcoinppl.cove.views.MaterialDivider
import org.bitcoinppl.cove.views.MaterialSection
import org.bitcoinppl.cove.views.SectionHeader
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
    val invalidUrlTitle = stringResource(R.string.ohttp_relay_invalid_url_title)
    val invalidUrlMessage = stringResource(R.string.ohttp_relay_invalid_url_message)
    val updateFailedTitle = stringResource(R.string.ohttp_relay_update_failed_title)
    val updateFailedMessage = stringResource(R.string.ohttp_relay_update_failed_message)
    val savedMessage = stringResource(R.string.ohttp_relay_saved)

    var relays by remember { mutableStateOf(config.ohttpRelayUrls()) }
    var newInput by remember { mutableStateOf("") }
    var isAdding by remember { mutableStateOf(false) }

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

    fun save(newRelays: List<String>, showSuccess: Boolean = true): Boolean {
        return try {
            relays = config.setOhttpRelayUrls(newRelays)
            newInput = ""
            isAdding = false
            keyboardController?.hide()
            if (showSuccess) scope.launch { snackbarHostState.showSnackbar(savedMessage) }
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

    fun addRelay() {
        val url = newInput.trim()
        if (url.isEmpty()) return
        save(relays + url)
    }

    fun deleteRelay(index: Int): Boolean {
        val updated = relays.toMutableList()
        updated.removeAt(index)
        return save(updated, showSuccess = false)
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
            SectionHeader(stringResource(R.string.ohttp_relay_description_title), showDivider = false)
            MaterialSection {
                Text(
                    text = stringResource(R.string.ohttp_relay_description),
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    modifier = Modifier.padding(horizontal = 16.dp, vertical = 12.dp),
                )
            }

            SectionHeader(stringResource(R.string.ohttp_relay_default_relays), showDivider = false)
            MaterialSection {
                Column(modifier = Modifier.padding(horizontal = 16.dp, vertical = 12.dp)) {
                    DEFAULT_RELAYS.forEach { relay ->
                        Text(
                            text = relay,
                            style = MaterialTheme.typography.bodySmall,
                            fontFamily = FontFamily.Monospace,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                    }
                }
            }

            SectionHeader(stringResource(R.string.ohttp_relay_custom_section))
            MaterialSection {
                Column {
                    relays.forEachIndexed { index, relay ->
                        RelayItem(
                            relay = relay,
                            onDelete = { deleteRelay(index) },
                        )
                        if (index < relays.lastIndex) {
                            MaterialDivider(indent = 16.dp)
                        }
                    }

                    if (isAdding) {
                        Row(
                            modifier =
                                Modifier
                                    .fillMaxWidth()
                                    .padding(horizontal = 16.dp, vertical = 8.dp),
                            verticalAlignment = Alignment.CenterVertically,
                        ) {
                            OutlinedTextField(
                                value = newInput,
                                onValueChange = { newInput = it },
                                label = {
                                    Text(stringResource(R.string.ohttp_relay_url_placeholder))
                                },
                                keyboardOptions =
                                    KeyboardOptions(
                                        capitalization = KeyboardCapitalization.None,
                                        imeAction = ImeAction.Done,
                                        keyboardType = KeyboardType.Uri,
                                    ),
                                keyboardActions = KeyboardActions(onDone = { addRelay() }),
                                singleLine = true,
                                modifier = Modifier.weight(1f),
                            )

                            TextButton(
                                onClick = ::addRelay,
                                enabled = newInput.trim().isNotEmpty(),
                            ) {
                                Text(stringResource(R.string.ohttp_relay_add))
                            }
                        }
                    } else {
                        TextButton(
                            onClick = { isAdding = true },
                            modifier = Modifier.padding(horizontal = 8.dp),
                        ) {
                            Text(stringResource(R.string.ohttp_relay_add_relay))
                        }
                    }
                }
            }
        }
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun RelayItem(
    relay: String,
    onDelete: () -> Boolean,
) {
    val dismissState =
        rememberSwipeToDismissBoxState(
            confirmValueChange = { value ->
                if (value == SwipeToDismissBoxValue.EndToStart) {
                    onDelete()
                } else {
                    false
                }
            },
            positionalThreshold = { it * 0.4f },
        )

    SwipeToDismissBox(
        state = dismissState,
        backgroundContent = {
            Box(
                modifier =
                    Modifier
                        .fillMaxSize()
                        .background(MaterialTheme.colorScheme.error)
                        .padding(end = 16.dp),
                contentAlignment = Alignment.CenterEnd,
            ) {
                Icon(
                    imageVector = Icons.Default.Delete,
                    contentDescription = null,
                    tint = MaterialTheme.colorScheme.onError,
                )
            }
        },
        enableDismissFromStartToEnd = false,
    ) {
        Row(
            modifier =
                Modifier
                    .fillMaxWidth()
                    .background(MaterialTheme.colorScheme.surface)
                    .padding(horizontal = 16.dp, vertical = 12.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Text(
                text = relay,
                style = MaterialTheme.typography.bodySmall,
                fontFamily = FontFamily.Monospace,
                modifier = Modifier.weight(1f),
            )
        }
    }
}
