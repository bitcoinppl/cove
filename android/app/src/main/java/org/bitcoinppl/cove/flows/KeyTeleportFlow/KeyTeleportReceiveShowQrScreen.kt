package org.bitcoinppl.cove.flows.KeyTeleportFlow

import android.content.ClipData
import android.content.ClipboardManager
import android.content.Context
import android.content.Intent
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.filled.ContentCopy
import androidx.compose.material.icons.filled.Share
import androidx.compose.material3.Button
import androidx.compose.material3.CenterAlignedTopAppBar
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.TopAppBarDefaults
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import org.bitcoinppl.cove.AppManager
import org.bitcoinppl.cove.QrCodeGenerator
import org.bitcoinppl.cove_core.KeyTeleportReceiverSession

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun KeyTeleportReceiveShowQrScreen(
    app: AppManager,
    session: KeyTeleportReceiverSession,
    onContinue: () -> Unit,
) {
    val context = LocalContext.current
    val bbqr = remember(session) { session.receiverPacketBbqr() }
    val code = remember(session) { session.numericCodeDisplay() }
    val qrBitmap = remember(bbqr) { QrCodeGenerator.generate(bbqr, 512) }

    fun copyToClipboard() {
        val clipboard = context.getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager
        clipboard.setPrimaryClip(ClipData.newPlainText("Key Teleport receiver packet", bbqr))
    }

    fun sharePacket() {
        val intent = Intent(Intent.ACTION_SEND).apply {
            type = "text/plain"
            putExtra(Intent.EXTRA_TEXT, bbqr)
        }
        context.startActivity(Intent.createChooser(intent, "Share Key Teleport receiver packet"))
    }

    Scaffold(
        topBar = {
            CenterAlignedTopAppBar(
                title = { Text("Key Teleport", fontWeight = FontWeight.SemiBold) },
                navigationIcon = {
                    IconButton(onClick = { app.popRoute() }) {
                        Icon(
                            Icons.AutoMirrored.Filled.ArrowBack,
                            contentDescription = "Back",
                        )
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
                .padding(horizontal = 24.dp)
                .verticalScroll(rememberScrollState()),
            horizontalAlignment = Alignment.CenterHorizontally,
            verticalArrangement = Arrangement.spacedBy(20.dp),
        ) {
            Spacer(modifier = Modifier.height(8.dp))

            Text(
                text = "Show this QR to the sender",
                style = MaterialTheme.typography.titleMedium,
                textAlign = TextAlign.Center,
            )

            Text(
                text = "The sender will scan your QR code, then you share your numeric code with them over a separate channel.",
                style = MaterialTheme.typography.bodyMedium,
                textAlign = TextAlign.Center,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )

            // QR code
            Image(
                bitmap = qrBitmap.asImageBitmap(),
                contentDescription = "Receiver QR code",
                modifier = Modifier
                    .size(260.dp)
                    .background(Color.White, RoundedCornerShape(12.dp))
                    .padding(12.dp),
            )

            // Copy / Share buttons for remote senders
            Row(
                horizontalArrangement = Arrangement.spacedBy(8.dp),
            ) {
                OutlinedButton(onClick = { copyToClipboard() }) {
                    Icon(
                        Icons.Filled.ContentCopy,
                        contentDescription = null,
                        modifier = Modifier.padding(end = 4.dp),
                    )
                    Text("Copy")
                }
                OutlinedButton(onClick = { sharePacket() }) {
                    Icon(
                        Icons.Filled.Share,
                        contentDescription = null,
                        modifier = Modifier.padding(end = 4.dp),
                    )
                    Text("Share")
                }
            }

            // numeric code display
            Column(
                horizontalAlignment = Alignment.CenterHorizontally,
                verticalArrangement = Arrangement.spacedBy(4.dp),
            ) {
                Text(
                    text = "Verification Code",
                    style = MaterialTheme.typography.labelMedium,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
                Box(
                    modifier = Modifier
                        .border(
                            1.dp,
                            MaterialTheme.colorScheme.outline,
                            RoundedCornerShape(8.dp),
                        )
                        .padding(horizontal = 20.dp, vertical = 12.dp),
                ) {
                    Row(horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                        Text(
                            text = code.take(4),
                            fontSize = 28.sp,
                            fontFamily = FontFamily.Monospace,
                            fontWeight = FontWeight.Bold,
                            letterSpacing = 4.sp,
                        )
                        Text(
                            text = code.drop(4),
                            fontSize = 28.sp,
                            fontFamily = FontFamily.Monospace,
                            fontWeight = FontWeight.Bold,
                            letterSpacing = 4.sp,
                        )
                    }
                }
                Text(
                    text = "Share this code over a separate channel (chat, phone call, etc.)",
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    textAlign = TextAlign.Center,
                )
            }

            Spacer(modifier = Modifier.weight(1f))

            Button(
                onClick = onContinue,
                modifier = Modifier
                    .fillMaxWidth()
                    .padding(bottom = 24.dp),
            ) {
                Text("Sender has scanned — Continue")
            }
        }
    }
}
