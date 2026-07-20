package org.bitcoinppl.cove.flows.keyteleport

import android.view.WindowManager
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.ContentPaste
import androidx.compose.material.icons.filled.QrCodeScanner
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.Icon
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import org.bitcoinppl.cove.QrCodeGenerator
import org.bitcoinppl.cove.ScreenSecurity
import org.bitcoinppl.cove.findActivity
import org.bitcoinppl.cove.ui.theme.CoveColor
import org.bitcoinppl.cove.views.RecoveryWordChip
import org.bitcoinppl.cove_core.KeyTeleportAlert
import org.bitcoinppl.cove_core.WalletMetadata
import org.bitcoinppl.cove_core.types.WalletId

private const val QR_BITMAP_SIZE = 720

@Composable
internal fun WalletChoices(
    wallets: List<WalletMetadata>,
    selectedWallet: WalletId?,
    onSelect: (WalletMetadata) -> Unit,
) {
    Column(verticalArrangement = Arrangement.spacedBy(10.dp)) {
        wallets.forEach { wallet ->
            val selected = selectedWallet == wallet.id
            OutlinedButton(
                onClick = { onSelect(wallet) },
                colors =
                    ButtonDefaults.outlinedButtonColors(
                        contentColor = if (selected) CoveColor.midnightBlue else Color.White,
                        containerColor = if (selected) CoveColor.btnPrimary else Color.Transparent,
                    ),
                modifier = Modifier.fillMaxWidth(),
            ) {
                Text(wallet.name, maxLines = 1)
            }
        }
    }
}

@Composable
internal fun PacketQr(text: String) {
    val bitmap = remember(text) { QrCodeGenerator.generate(text, size = QR_BITMAP_SIZE) }

    Box(modifier = Modifier.fillMaxWidth(), contentAlignment = Alignment.Center) {
        Image(
            bitmap = bitmap.asImageBitmap(),
            contentDescription = "Key Teleport QR",
            contentScale = ContentScale.Fit,
            modifier =
                Modifier
                    .size(280.dp)
                    .clip(RoundedCornerShape(12.dp))
                    .background(Color.White)
                    .padding(12.dp),
        )
    }
}

@Composable
internal fun RecoveryWordsGrid(words: List<String>) {
    Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
        words.chunked(2).forEachIndexed { rowIndex, rowWords ->
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                rowWords.forEachIndexed { columnIndex, word ->
                    RecoveryWordChip(
                        index = rowIndex * 2 + columnIndex + 1,
                        word = word,
                        modifier = Modifier.weight(1f),
                    )
                }
                if (rowWords.size == 1) Spacer(Modifier.weight(1f))
            }
        }
    }
}

@Composable
internal fun TextBlock(
    title: String,
    body: String,
) {
    Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
        Text(text = title, color = Color.White, fontSize = 26.sp, fontWeight = FontWeight.SemiBold)
        Text(text = body, color = Color.White.copy(alpha = 0.74f), lineHeight = 20.sp)
    }
}

@Composable
internal fun ActionRow(
    onScan: () -> Unit,
    onPaste: () -> Unit,
) {
    Row(horizontalArrangement = Arrangement.spacedBy(12.dp)) {
        Button(onClick = onScan, modifier = Modifier.weight(1f)) {
            Icon(Icons.Default.QrCodeScanner, contentDescription = null)
            Spacer(Modifier.size(8.dp))
            Text("Scan")
        }
        OutlinedButton(onClick = onPaste, modifier = Modifier.weight(1f)) {
            Icon(Icons.Default.ContentPaste, contentDescription = null)
            Spacer(Modifier.size(8.dp))
            Text("Paste")
        }
    }
}

@Composable
internal fun LoadingText(text: String) {
    Box(
        modifier = Modifier.fillMaxWidth().height(180.dp),
        contentAlignment = Alignment.Center,
    ) {
        Text(text, color = Color.White.copy(alpha = 0.74f))
    }
}

@Composable
internal fun SecretCode(value: String) {
    Text(
        text = value,
        color = Color.White,
        fontSize = 28.sp,
        fontWeight = FontWeight.SemiBold,
        modifier = Modifier.fillMaxWidth(),
        textAlign = TextAlign.Center,
    )
}

@Composable
internal fun SecureScreenEffect() {
    val context = LocalContext.current
    DisposableEffect(Unit) {
        val window = context.findActivity()?.window
        ScreenSecurity.enter()
        window?.setFlags(
            WindowManager.LayoutParams.FLAG_SECURE,
            WindowManager.LayoutParams.FLAG_SECURE,
        )
        onDispose {
            ScreenSecurity.exit()
            if (!ScreenSecurity.isSensitiveScreen) window?.clearFlags(WindowManager.LayoutParams.FLAG_SECURE)
        }
    }
}

@Composable
internal fun KeyTeleportAlertDialog(
    alert: KeyTeleportAlert,
    onDismiss: () -> Unit,
) {
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text("Key Teleport") },
        text = { Text(alert.messageForDisplay()) },
        confirmButton = { TextButton(onClick = onDismiss) { Text("OK") } },
    )
}

@Composable
internal fun KeyTeleportErrorDialog(
    message: String,
    onDismiss: () -> Unit,
) {
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text("Key Teleport") },
        text = { Text(message) },
        confirmButton = { TextButton(onClick = onDismiss) { Text("OK") } },
    )
}
