package org.bitcoinppl.cove.flows.KeyTeleportFlow

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.filled.Visibility
import androidx.compose.material.icons.filled.VisibilityOff
import androidx.compose.material3.Button
import androidx.compose.material3.CenterAlignedTopAppBar
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.TopAppBarDefaults
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.text.input.VisualTransformation
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import org.bitcoinppl.cove.AppManager
import org.bitcoinppl.cove.Log
import org.bitcoinppl.cove.TaggedItem
import org.bitcoinppl.cove_core.AppAlertState
import org.bitcoinppl.cove_core.KeyTeleportException
import org.bitcoinppl.cove_core.KeyTeleportPayload
import org.bitcoinppl.cove_core.KeyTeleportPayloadKind
import org.bitcoinppl.cove_core.KeyTeleportReceiverSession

private const val TAG = "KeyTeleportPasswordScreen"

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun KeyTeleportReceivePasswordScreen(
    app: AppManager,
    session: KeyTeleportReceiverSession,
    senderPacketBbqr: String,
    onDecoded: (KeyTeleportPayloadKind, String) -> Unit,
) {
    var password by remember { mutableStateOf("") }
    var showPassword by remember { mutableStateOf(false) }
    var isDecoding by remember { mutableStateOf(false) }
    val scope = rememberCoroutineScope()

    fun tryDecode() {
        if (password.isBlank() || isDecoding) return
        isDecoding = true
        scope.launch {
            try {
                val payload = withContext(Dispatchers.Default) {
                    session.decode(senderPacketBbqr, password)
                }
                val (kind, data) = when (payload) {
                    is KeyTeleportPayload.Mnemonic -> KeyTeleportPayloadKind.MNEMONIC to payload.words
                    is KeyTeleportPayload.Xprv -> KeyTeleportPayloadKind.XPRV to payload.xprv
                }
                onDecoded(kind, data)
            } catch (e: KeyTeleportException.DecodeFailed) {
                Log.w(TAG, "Wrong password or code: $e")
                app.alertState = TaggedItem(
                    AppAlertState.General(
                        title = "Wrong Password",
                        message = "The teleport password is incorrect. Check with the sender and try again.",
                    ),
                )
            } catch (e: KeyTeleportException.InvalidSenderPacket) {
                Log.w(TAG, "Invalid sender packet: $e")
                app.alertState = TaggedItem(
                    AppAlertState.General(
                        title = "Invalid Packet",
                        message = "The scanned QR code is not a valid Key Teleport sender packet.",
                    ),
                )
            } catch (e: Exception) {
                Log.e(TAG, "Unexpected decode error", e)
                app.alertState = TaggedItem(
                    AppAlertState.General(
                        title = "Error",
                        message = e.message ?: "Unknown error",
                    ),
                )
            } finally {
                isDecoding = false
            }
        }
    }

    Scaffold(
        topBar = {
            CenterAlignedTopAppBar(
                title = { Text("Enter Password", fontWeight = FontWeight.SemiBold) },
                navigationIcon = {
                    IconButton(onClick = { app.popRoute() }) {
                        Icon(Icons.AutoMirrored.Filled.ArrowBack, contentDescription = "Back")
                    }
                },
                colors = TopAppBarDefaults.centerAlignedTopAppBarColors(
                    containerColor = Color.Transparent,
                ),
            )
        },
    ) { paddingValues ->
        Column(
            modifier = Modifier
                .fillMaxSize()
                .padding(paddingValues)
                .padding(horizontal = 24.dp),
            horizontalAlignment = Alignment.CenterHorizontally,
            verticalArrangement = Arrangement.spacedBy(16.dp),
        ) {
            Spacer(modifier = Modifier.height(12.dp))

            Text(
                text = "Enter the teleport password",
                style = MaterialTheme.typography.titleMedium,
                textAlign = TextAlign.Center,
            )

            Text(
                text = "The sender shared a one-time password alongside the QR code. Enter it here to decrypt the received secret.",
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                textAlign = TextAlign.Center,
            )

            OutlinedTextField(
                value = password,
                onValueChange = { password = it },
                label = { Text("Teleport password") },
                visualTransformation = if (showPassword) {
                    VisualTransformation.None
                } else {
                    PasswordVisualTransformation()
                },
                trailingIcon = {
                    IconButton(onClick = { showPassword = !showPassword }) {
                        Icon(
                            if (showPassword) Icons.Filled.VisibilityOff else Icons.Filled.Visibility,
                            contentDescription = if (showPassword) "Hide password" else "Show password",
                        )
                    }
                },
                singleLine = true,
                keyboardOptions = KeyboardOptions(imeAction = ImeAction.Done),
                keyboardActions = KeyboardActions(onDone = { tryDecode() }),
                modifier = Modifier.fillMaxWidth(),
            )

            Spacer(modifier = Modifier.weight(1f))

            Button(
                onClick = { tryDecode() },
                enabled = password.isNotBlank() && !isDecoding,
                modifier = Modifier
                    .fillMaxWidth()
                    .padding(bottom = 24.dp),
            ) {
                Text(if (isDecoding) "Decrypting…" else "Decrypt")
            }
        }
    }
}
