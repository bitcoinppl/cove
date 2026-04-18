package org.bitcoinppl.cove.flows.KeyTeleportFlow

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.TopAppBar
import androidx.compose.material3.TopAppBarDefaults
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.font.FontWeight
import org.bitcoinppl.cove.AppManager
import org.bitcoinppl.cove.QrCodeScanView
import org.bitcoinppl.cove.TaggedItem
import org.bitcoinppl.cove_core.AppAlertState
import org.bitcoinppl.cove_core.MultiFormat

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun KeyTeleportReceiveScanSenderScreen(
    app: AppManager,
    onScanned: (senderPacketBbqr: String) -> Unit,
) {
    Scaffold(
        topBar = {
            TopAppBar(
                title = {
                    Text("Scan Sender QR", fontWeight = FontWeight.SemiBold, color = Color.White)
                },
                navigationIcon = {
                    IconButton(onClick = { app.popRoute() }) {
                        Icon(
                            Icons.AutoMirrored.Filled.ArrowBack,
                            contentDescription = "Back",
                            tint = Color.White,
                        )
                    }
                },
                colors = TopAppBarDefaults.topAppBarColors(containerColor = Color.Transparent),
            )
        },
        containerColor = Color.Black,
        modifier = Modifier.fillMaxSize(),
    ) { paddingValues ->
        Box(modifier = Modifier.fillMaxSize().padding(paddingValues)) {
            QrCodeScanView(
                showTopBar = false,
                app = app,
                onDismiss = { app.popRoute() },
                onScanned = { multiFormat ->
                    when (multiFormat) {
                        is MultiFormat.KeyTeleportSenderPacket -> {
                            onScanned(multiFormat.v1)
                        }
                        else -> {
                            app.alertState = TaggedItem(
                                AppAlertState.General(
                                    title = "Wrong QR Code",
                                    message = "Please scan the sender's Key Teleport QR code (starts with B\$2S)",
                                ),
                            )
                        }
                    }
                },
            )
        }
    }
}
