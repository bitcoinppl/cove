package org.bitcoinppl.cove.flows.keyteleport

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.Note
import androidx.compose.material.icons.filled.Check
import androidx.compose.material.icons.filled.ContentCopy
import androidx.compose.material.icons.filled.Key
import androidx.compose.material.icons.filled.QrCodeScanner
import androidx.compose.material.icons.filled.Visibility
import androidx.compose.material.icons.filled.VisibilityOff
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import org.bitcoinppl.cove.flows.OnboardingFlow.OnboardingStatusHero
import org.bitcoinppl.cove.flows.OnboardingFlow.OnboardingTextSecondary
import org.bitcoinppl.cove.flows.OnboardingFlow.OnboardingPrimaryButton
import org.bitcoinppl.cove.ui.theme.CoveColor
import org.bitcoinppl.cove_core.KeyTeleportManagerAction
import org.bitcoinppl.cove_core.KeyTeleportMessageItem
import org.bitcoinppl.cove_core.KeyTeleportMessageReview
import org.bitcoinppl.cove_core.KeyTeleportReceiveState
import org.bitcoinppl.cove_core.KeyTeleportXprvReview
import org.bitcoinppl.cove_core.WalletMetadata

private val ImportedWalletSuccessTint = Color(0xFF7DD195)
private val ImportedWalletSuccessFill = Color(0x297DD195)

@Composable
internal fun ReceiveReadyView(
    receive: KeyTeleportReceiveState,
    onScan: () -> Unit,
) {
    val packetText = remember(receive.packet) { runCatching { receive.packet.bbqrPart() }.getOrNull() }

    if (packetText == null) {
        Text("Unable to render this receive request.", color = MaterialTheme.colorScheme.error)
        ReceiverCode(receive.groupedNumericCode)
    } else {
        KeyTeleportRevealPair(
            qrHint = "Tap to show QR code",
            codeHint = "Tap to show receiver code",
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
internal fun ReceiveMnemonicReviewView(
    manager: KeyTeleportManager,
    wordCount: Int,
    onDone: () -> Unit,
) {
    var words by remember { mutableStateOf<List<String>?>(null) }

    LaunchedEffect(wordCount) { words = manager.revealMnemonicWords() }
    DisposableEffect(Unit) {
        onDispose { words = emptyList() }
    }
    SecureScreenEffect()

    TextBlock(
        title = "Recovery words received",
        body = "Cove found a $wordCount-word wallet. Review it below or import it directly.",
    )
    val revealedWords = words
    if (revealedWords == null || revealedWords.isEmpty()) {
        Text("Unable to reveal recovery words.", color = MaterialTheme.colorScheme.error)
    } else {
        RecoveryWordsGrid(revealedWords)
    }
    Button(
        enabled = !revealedWords.isNullOrEmpty(),
        onClick = {
            words = emptyList()
            manager.dispatch(KeyTeleportManagerAction.ImportReceivedWallet)
        },
        modifier = Modifier.fillMaxWidth(),
    ) {
        Text("Import Wallet")
    }
    TextButton(
        onClick = {
            words = emptyList()
            manager.dispatch(KeyTeleportManagerAction.FinishReview)
            onDone()
        },
        colors = ButtonDefaults.textButtonColors(contentColor = MaterialTheme.colorScheme.error),
        modifier = Modifier.fillMaxWidth(),
    ) {
        Text("Finish Without Importing")
    }
}

@Composable
internal fun ReceiveXprvReviewView(
    manager: KeyTeleportManager,
    review: KeyTeleportXprvReview,
    onDone: () -> Unit,
) {
    var xprv by remember { mutableStateOf<String?>(null) }

    SecureScreenEffect()
    DisposableEffect(Unit) {
        onDispose {
            xprv = null
            manager.dispatch(KeyTeleportManagerAction.HideXprv)
        }
    }
    LaunchedEffect(review.revealed) {
        xprv = if (review.revealed) manager.revealXprv() else null
    }

    TextBlock(
        title = "Extended private key received",
        body = "Import this key as a hot wallet, or reveal it only when you are ready to handle it.",
    )
    XprvRevealContent(manager, review.revealed, xprv)
    Button(
        onClick = {
            xprv = null
            manager.dispatch(KeyTeleportManagerAction.ImportReceivedWallet)
        },
        modifier = Modifier.fillMaxWidth(),
    ) {
        Text("Import Wallet")
    }
    TextButton(
        onClick = {
            xprv = null
            manager.dispatch(KeyTeleportManagerAction.FinishReview)
            onDone()
        },
        modifier = Modifier.fillMaxWidth(),
    ) {
        Text("Finish Without Importing")
    }
}

@Composable
private fun XprvRevealContent(
    manager: KeyTeleportManager,
    revealed: Boolean,
    xprv: String?,
) {
    val context = LocalContext.current
    if (!revealed) {
        Button(
            onClick = { manager.dispatch(KeyTeleportManagerAction.RevealXprv) },
            modifier = Modifier.fillMaxWidth(),
        ) {
            Icon(Icons.Default.Visibility, contentDescription = null)
            Spacer(Modifier.size(8.dp))
            Text("Reveal")
        }
        return
    }

    if (xprv == null) {
        Text("Unable to reveal this extended private key.", color = MaterialTheme.colorScheme.error)
        return
    }

    SelectionContainer {
        Text(
            text = xprv,
            color = Color.White,
            modifier =
                Modifier
                    .fillMaxWidth()
                    .clip(RoundedCornerShape(8.dp))
                    .background(Color.White.copy(alpha = 0.08f))
                    .padding(12.dp),
        )
    }
    Row(horizontalArrangement = Arrangement.spacedBy(12.dp)) {
        OutlinedButton(
            onClick = { copyText(context, "Key Teleport xprv", xprv, sensitive = true) },
            modifier = Modifier.weight(1f),
        ) {
            Icon(Icons.Default.ContentCopy, contentDescription = null)
            Spacer(Modifier.size(8.dp))
            Text("Copy")
        }
        OutlinedButton(
            onClick = { manager.dispatch(KeyTeleportManagerAction.HideXprv) },
            modifier = Modifier.weight(1f),
        ) {
            Icon(Icons.Default.VisibilityOff, contentDescription = null)
            Spacer(Modifier.size(8.dp))
            Text("Hide")
        }
    }
}

@Composable
internal fun ReceiveMessageReviewView(
    manager: KeyTeleportManager,
    review: KeyTeleportMessageReview,
    onDone: () -> Unit,
) {
    SecureScreenEffect()
    TextBlock(
        title = if (review.items.size == 1) "Message received" else "Messages received",
        body = "This transfer contains text, not a wallet. Cove displays it exactly as received.",
    )
    review.items.forEach { MessageItemCard(it) }
    Button(
        onClick = {
            manager.dispatch(KeyTeleportManagerAction.FinishReview)
            onDone()
        },
        modifier = Modifier.fillMaxWidth(),
    ) {
        Text("Done")
    }
}

@Composable
private fun MessageItemCard(item: KeyTeleportMessageItem) {
    Surface(
        color = CoveColor.midnightBlue.copy(alpha = 0.5f),
        shape = RoundedCornerShape(14.dp),
        modifier = Modifier.fillMaxWidth(),
    ) {
        Column(
            modifier = Modifier.padding(16.dp),
            verticalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            val password = item is KeyTeleportMessageItem.Password
            val (title, group) =
                when (item) {
                    is KeyTeleportMessageItem.Note -> item.title to item.group
                    is KeyTeleportMessageItem.Password -> item.title to item.group
                }
            Row(verticalAlignment = Alignment.CenterVertically) {
                Icon(
                    imageVector = if (password) Icons.Default.Key else Icons.AutoMirrored.Filled.Note,
                    contentDescription = null,
                    tint = CoveColor.btnPrimary,
                )
                Spacer(Modifier.size(10.dp))
                Text(title, color = Color.White, fontWeight = FontWeight.SemiBold, modifier = Modifier.weight(1f))
                if (group.isNotEmpty()) Text(group, color = OnboardingTextSecondary)
            }
            when (item) {
                is KeyTeleportMessageItem.Note -> {
                    MessageField("Message", item.text)
                }

                is KeyTeleportMessageItem.Password -> {
                    MessageField("Username", item.username)
                    MessageField("Password", item.password)
                    MessageField("Website", item.site)
                    MessageField("Notes", item.notes)
                }
            }
        }
    }
}

@Composable
private fun MessageField(
    label: String,
    value: String,
) {
    if (value.isEmpty()) return

    Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
        Text(
            text = label.uppercase(),
            color = Color.White.copy(alpha = 0.56f),
            style = MaterialTheme.typography.labelSmall,
            fontWeight = FontWeight.SemiBold,
        )
        SelectionContainer {
            Text(value, color = Color.White, modifier = Modifier.fillMaxWidth())
        }
    }
}

@Composable
internal fun ReceiveImportedWalletView(
    manager: KeyTeleportManager,
    wallet: WalletMetadata,
    title: String = "Wallet imported",
    message: String = "${wallet.name} is ready to use in Cove.",
    buttonTitle: String = "Done",
    onDone: () -> Unit,
) {
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
        Text(title, color = Color.White, fontSize = 26.sp, fontWeight = FontWeight.SemiBold)
        Text(message, color = OnboardingTextSecondary)
        Button(
            onClick = {
                manager.dispatch(KeyTeleportManagerAction.Clear)
                onDone()
            },
            modifier = Modifier.fillMaxWidth(),
        ) {
            Text(buttonTitle)
        }
    }
}
