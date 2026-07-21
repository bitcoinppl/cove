package org.bitcoinppl.cove.flows.keyteleport

import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material3.Button
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.input.KeyboardType
import org.bitcoinppl.cove.AppManager
import org.bitcoinppl.cove_core.KeyTeleportManagerAction
import org.bitcoinppl.cove_core.KeyTeleportSendChooseWallet
import org.bitcoinppl.cove_core.KeyTeleportSendConfirm
import org.bitcoinppl.cove_core.KeyTeleportSendEnterCode
import org.bitcoinppl.cove_core.KeyTeleportSendReady

private const val RECEIVER_CODE_LENGTH = 8

@Composable
internal fun SendIdleView(
    manager: KeyTeleportManager,
    app: AppManager,
    onScan: () -> Unit,
    onPaste: () -> Unit,
) {
    TextBlock(
        title = "Send a wallet",
        body = "Scan or paste the receiver code, then choose a hot wallet to send.",
    )
    ActionRow(onScan, onPaste)

    val eligibleWallets = remember(app.wallets) { app.wallets.filter { app.canKeyTeleportSend(it.id) } }
    if (eligibleWallets.isEmpty()) {
        Text("No eligible hot wallets are available on this device.", color = Color.White.copy(alpha = 0.75f))
    } else {
        WalletChoices(eligibleWallets, selectedWallet = null) {
            manager.dispatch(KeyTeleportManagerAction.StartSendFromWallet(it.id))
        }
    }
}

@Composable
internal fun SendChooseWalletView(
    manager: KeyTeleportManager,
    choose: KeyTeleportSendChooseWallet,
) {
    TextBlock(
        title = "Choose wallet",
        body = "Select the hot wallet to send.",
    )
    WalletChoices(choose.eligibleWallets, selectedWallet = null) {
        manager.dispatch(KeyTeleportManagerAction.SelectSendWallet(it.id))
    }
}

@Composable
internal fun SendAwaitReceiverView(
    onScan: () -> Unit,
    onPaste: () -> Unit,
) {
    TextBlock(
        title = "Scan Receiver Request",
        body = "Scan or paste the request shown on the receiving device.",
    )
    ActionRow(onScan, onPaste)
}

@Composable
internal fun SendEnterCodeView(
    manager: KeyTeleportManager,
    send: KeyTeleportSendEnterCode,
) {
    var code by remember { mutableStateOf("") }

    TextBlock(
        title = "Enter receiver code",
        body = "Use the numeric receiver code shown on the receiving device for ${send.selectedWallet.name}.",
    )
    OutlinedTextField(
        value = code,
        onValueChange = { code = it.filter(Char::isDigit).take(RECEIVER_CODE_LENGTH) },
        label = { Text("Receiver code") },
        singleLine = true,
        keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Number),
        colors = keyTeleportTextFieldColors(),
        modifier = Modifier.fillMaxWidth(),
    )
    Button(
        enabled = code.isNotBlank(),
        onClick = { manager.dispatch(KeyTeleportManagerAction.EnterReceiverCode(code)) },
        modifier = Modifier.fillMaxWidth(),
    ) {
        Text("Continue")
    }
}

@Composable
internal fun SendConfirmView(
    manager: KeyTeleportManager,
    confirm: KeyTeleportSendConfirm,
) {
    TextBlock(
        title = "Confirm send",
        body = "Key Teleport will create an encrypted transfer for ${confirm.selectedWallet.name}.",
    )
    Button(
        onClick = { manager.dispatch(KeyTeleportManagerAction.ConfirmSendWallet) },
        modifier = Modifier.fillMaxWidth(),
    ) {
        Text("Create Sender Code")
    }
}

@Composable
internal fun SendReadyView(
    ready: KeyTeleportSendReady,
    onDone: () -> Unit,
) {
    val packetText = remember(ready.packet) { ready.packet.bbqrPart() }
    val password = remember(ready.password) { ready.password.groupedText() }

    SecureScreenEffect()
    TextBlock(
        title = "Sender code ready",
        body = "Show this QR to the receiver, then read the password to complete the transfer.",
    )
    PacketQr(packetText)
    SecretCode(password)
    Button(onClick = onDone, modifier = Modifier.fillMaxWidth()) {
        Text("Done")
    }
}
