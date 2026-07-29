package org.bitcoinppl.cove.flows.keyteleport

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Check
import androidx.compose.material.icons.filled.QrCodeScanner
import androidx.compose.material3.Button
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import org.bitcoinppl.cove.flows.OnboardingFlow.OnboardingPrimaryButton
import org.bitcoinppl.cove.flows.OnboardingFlow.OnboardingStatusHero
import org.bitcoinppl.cove.flows.OnboardingFlow.OnboardingTextSecondary
import org.bitcoinppl.cove_core.KeyTeleportManagerAction
import org.bitcoinppl.cove_core.KeyTeleportReceiveState
import org.bitcoinppl.cove_core.WalletMetadata

private val ImportedWalletSuccessTint = Color(0xFF7DD195)
private val ImportedWalletSuccessFill = Color(0x297DD195)

internal enum class ImportedWalletStatus {
    IMPORTED,
    ALREADY_IMPORTED,
}

private data class ImportedWalletContent(
    val title: String,
    val message: String,
    val buttonTitle: String,
)

@Composable
internal fun ReceiveReadyView(
    receive: KeyTeleportReceiveState,
    concealmentGeneration: Long,
    onScan: () -> Unit,
) {
    val packetText = remember(receive.packet) { runCatching { receive.packet.bbqrPart() }.getOrNull() }

    SecureScreenEffect()
    if (packetText == null) {
        Text("Unable to render this receive request.", color = MaterialTheme.colorScheme.error)
        ReceiverCode(receive.groupedNumericCode)
    } else {
        KeyTeleportRevealPair(
            qrHint = "Tap to show QR code",
            codeHint = "Tap to show receiver code",
            resetKey = concealmentGeneration,
            qr = { PacketQr(packetText) },
            code = { ReceiverCode(receive.groupedNumericCode) },
        )
    }
    Text(
        text =
            "Have the sending wallet scan the QR code, then send the receiver code through a different " +
                "channel, such as a call or message.\n\n" +
                "If the sending wallet cannot scan this screen, tap Share and open the link on another " +
                "device. The link shows the same QR code.",
        color = OnboardingTextSecondary,
        style = MaterialTheme.typography.bodySmall,
    )
    OnboardingPrimaryButton(
        text = "Scan Sender Response",
        onClick = onScan,
        icon = Icons.Default.QrCodeScanner,
    )
}

@Composable
private fun ReceiverCode(code: String) {
    Column(
        modifier = Modifier.fillMaxWidth(),
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.spacedBy(4.dp),
    ) {
        Text(
            text = "Receiver Code",
            color = OnboardingTextSecondary,
            style = MaterialTheme.typography.labelMedium,
        )
        KeyTeleportCodeText(code)
    }
}

@Composable
internal fun ReceivePasswordView(manager: KeyTeleportManager) {
    var password by remember { mutableStateOf("") }

    SecureScreenEffect()
    TextBlock(
        title = "Enter sender password",
        body = "Type the password shown by the sending device.",
    )
    OutlinedTextField(
        value = password,
        onValueChange = { password = it },
        label = { Text("Password") },
        singleLine = true,
        keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Password),
        visualTransformation = PasswordVisualTransformation(),
        colors = keyTeleportTextFieldColors(),
        modifier = Modifier.fillMaxWidth(),
    )
    Button(
        enabled = password.isNotBlank(),
        onClick = { manager.dispatch(KeyTeleportManagerAction.EnterSenderPassword(password.trim())) },
        modifier = Modifier.fillMaxWidth(),
    ) {
        Text("Continue")
    }
}

@Composable
internal fun ReceiveImportedWalletView(
    manager: KeyTeleportManager,
    wallet: WalletMetadata,
    status: ImportedWalletStatus,
    onDone: () -> Unit,
) {
    val content =
        when (status) {
            ImportedWalletStatus.IMPORTED ->
                ImportedWalletContent(
                    title = "Wallet imported",
                    message = "${wallet.name} is ready to use in Cove.",
                    buttonTitle = "Done",
                )

            ImportedWalletStatus.ALREADY_IMPORTED ->
                ImportedWalletContent(
                    title = "Wallet already imported",
                    message = "${wallet.name} is already available in Cove.",
                    buttonTitle = "Open Wallet",
                )
        }

    Column(
        modifier = Modifier.fillMaxWidth(),
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.spacedBy(18.dp),
    ) {
        OnboardingStatusHero(
            icon = Icons.Default.Check,
            tint = ImportedWalletSuccessTint,
            fillColor = ImportedWalletSuccessFill,
        )
        Text(content.title, color = Color.White, fontSize = 26.sp, fontWeight = FontWeight.SemiBold)
        Text(content.message, color = OnboardingTextSecondary)
        Button(
            onClick = {
                manager.dispatch(KeyTeleportManagerAction.Clear)
                onDone()
            },
            modifier = Modifier.fillMaxWidth(),
        ) {
            Text(content.buttonTitle)
        }
    }
}
