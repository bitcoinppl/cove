package org.bitcoinppl.cove.flows.SettingsFlow

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
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Button
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Scaffold
import androidx.compose.material3.SnackbarHost
import androidx.compose.material3.SnackbarHostState
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
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
import org.bitcoinppl.cove.views.MaterialSection
import org.bitcoinppl.cove.views.SectionHeader
import org.bitcoinppl.cove_core.AppAlertState
import org.bitcoinppl.cove_core.Database

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
    val scope = rememberCoroutineScope()
    val snackbarHostState = remember { SnackbarHostState() }
    val keyboardController = LocalSoftwareKeyboardController.current
    val savedMessage = stringResource(R.string.ohttp_relay_saved)
    val invalidUrlTitle = stringResource(R.string.ohttp_relay_invalid_url_title)
    val invalidUrlMessage = stringResource(R.string.ohttp_relay_invalid_url_message)
    val updateFailedTitle = stringResource(R.string.ohttp_relay_update_failed_title)
    val updateFailedMessage = stringResource(R.string.ohttp_relay_update_failed_message)

    var input by remember { mutableStateOf(config.ohttpRelayUrl() ?: "") }
    var isSaving by remember { mutableStateOf(false) }

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

    fun save() {
        if (isSaving) return

        val inputToSave = input
        scope.launch {
            isSaving = true
            try {
                val normalized = config.setOhttpRelayUrl(inputToSave)
                input = normalized ?: ""
                keyboardController?.hide()
                isSaving = false

                launch {
                    snackbarHostState.showSnackbar(savedMessage)
                }
            } catch (e: Exception) {
                showAlert(invalidUrlTitle, invalidUrlMessage)
                isSaving = false
            }
        }
    }

    fun reset() {
        try {
            config.clearOhttpRelayUrl()
            input = ""
        } catch (e: Exception) {
            showAlert(updateFailedTitle, updateFailedMessage)
        }
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
                Column(modifier = Modifier.padding(horizontal = 16.dp, vertical = 12.dp)) {
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
                        keyboardActions = KeyboardActions(onDone = { save() }),
                        singleLine = true,
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
                            Text(stringResource(R.string.ohttp_relay_save))
                        }

                        TextButton(onClick = ::reset) {
                            Text(stringResource(R.string.ohttp_relay_reset))
                        }
                    }
                }
            }
        }
    }
}
