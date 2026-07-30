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
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.Note
import androidx.compose.material.icons.filled.ContentCopy
import androidx.compose.material.icons.filled.Key
import androidx.compose.material.icons.filled.Visibility
import androidx.compose.material.icons.filled.VisibilityOff
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
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
import androidx.compose.ui.unit.dp
import org.bitcoinppl.cove.flows.OnboardingFlow.OnboardingTextSecondary
import org.bitcoinppl.cove.ui.theme.CoveColor
import org.bitcoinppl.cove_core.KeyTeleportManagerAction
import org.bitcoinppl.cove_core.KeyTeleportMessageItem
import org.bitcoinppl.cove_core.KeyTeleportMessageReview
import org.bitcoinppl.cove_core.KeyTeleportXprvReview

private const val MNEMONIC_PLACEHOLDER_WORD_COUNT = 4

private sealed interface MnemonicDisclosure {
    data object Hidden : MnemonicDisclosure

    data class Revealed(
        val words: List<String>,
    ) : MnemonicDisclosure

    data object Failed : MnemonicDisclosure
}

@Composable
internal fun ReceiveMnemonicReviewView(
    manager: KeyTeleportManager,
    wordCount: Int,
    onDone: () -> Unit,
) {
    var disclosure by remember(wordCount, manager.concealmentGeneration) {
        mutableStateOf<MnemonicDisclosure>(MnemonicDisclosure.Hidden)
    }

    DisposableEffect(Unit) {
        onDispose { disclosure = MnemonicDisclosure.Hidden }
    }
    SecureScreenEffect()

    TextBlock(
        title = "Recovery words received",
        body = "Cove found a $wordCount-word wallet. Review it below or import it directly.",
    )
    val displayedWords =
        when (val currentDisclosure = disclosure) {
            MnemonicDisclosure.Hidden -> List(MNEMONIC_PLACEHOLDER_WORD_COUNT) { "••••••" }
            is MnemonicDisclosure.Revealed -> currentDisclosure.words
            MnemonicDisclosure.Failed -> null
        }
    if (displayedWords == null) {
        Text("Unable to reveal recovery words.", color = MaterialTheme.colorScheme.error)
    } else {
        KeyTeleportRevealable(
            isHidden = disclosure == MnemonicDisclosure.Hidden,
            hint = "Tap to reveal recovery words",
            blurRadius = 10.dp,
            onReveal = {
                val words = manager.secrets.revealMnemonicWords()
                disclosure =
                    if (words.isNullOrEmpty()) {
                        MnemonicDisclosure.Failed
                    } else {
                        MnemonicDisclosure.Revealed(words)
                    }
            },
        ) {
            RecoveryWordsGrid(displayedWords)
        }
    }
    Button(
        onClick = {
            disclosure = MnemonicDisclosure.Hidden
            manager.dispatch(KeyTeleportManagerAction.ImportReceivedWallet)
        },
        modifier = Modifier.fillMaxWidth(),
    ) {
        Text("Import Wallet")
    }
    TextButton(
        onClick = {
            disclosure = MnemonicDisclosure.Hidden
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
        xprv = if (review.revealed) manager.secrets.revealXprv() else null
    }
    LaunchedEffect(manager.concealmentGeneration) {
        xprv = null
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
            onClick = { copyText(context, "KeyTeleport xprv", xprv, sensitive = true) },
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
