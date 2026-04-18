package org.bitcoinppl.cove.flows.KeyTeleportFlow

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material3.Button
import androidx.compose.material3.CenterAlignedTopAppBar
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.TopAppBarDefaults
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import org.bitcoinppl.cove.AppManager
import org.bitcoinppl.cove.ImportWalletManager
import org.bitcoinppl.cove.Log
import org.bitcoinppl.cove.TaggedItem
import org.bitcoinppl.cove_core.AppAlertState
import org.bitcoinppl.cove_core.ImportWalletException
import org.bitcoinppl.cove_core.KeyTeleportPayload
import org.bitcoinppl.cove_core.Route

private const val TAG = "KeyTeleportImportScreen"

@OptIn(ExperimentalMaterial3Api::class, ExperimentalLayoutApi::class)
@Composable
fun KeyTeleportReceiveImportScreen(
    app: AppManager,
    payload: KeyTeleportPayload,
) {
    var isImporting by remember { mutableStateOf(false) }

    val isMnemonic = payload is KeyTeleportPayload.Mnemonic
    val words = if (payload is KeyTeleportPayload.Mnemonic) payload.words.split(" ") else emptyList()

    fun doImport() {
        if (isImporting) return
        isImporting = true

        try {
            when (payload) {
                is KeyTeleportPayload.Mnemonic -> {
                    val metadata = try {
                        val manager = ImportWalletManager()
                        try {
                            manager.importWallet(listOf(words))
                        } finally {
                            manager.close()
                        }
                    } catch (e: ImportWalletException.WalletAlreadyExists) {
                        Log.w(TAG, "Wallet already exists: ${e.v1}")
                        app.alertState = TaggedItem(AppAlertState.DuplicateWallet(e.v1))
                        return
                    }
                    app.rust.selectWallet(metadata.id)
                    app.resetRoute(Route.SelectedWallet(metadata.id))
                    app.alertState = TaggedItem(AppAlertState.ImportedSuccessfully)
                }

                is KeyTeleportPayload.Xprv -> {
                    // XPRV hot-wallet import is not yet supported in this version.
                    app.alertState = TaggedItem(
                        AppAlertState.General(
                            title = "Not Yet Supported",
                            message = "Importing an XPRV via Key Teleport is not yet supported. Only mnemonic transfer is supported at this time.",
                        ),
                    )
                }
            }
        } catch (e: Exception) {
            Log.e(TAG, "Import failed", e)
            app.alertState = TaggedItem(
                AppAlertState.General(
                    title = "Import Failed",
                    message = e.message ?: "Unknown error",
                ),
            )
        } finally {
            isImporting = false
        }
    }

    Scaffold(
        topBar = {
            CenterAlignedTopAppBar(
                title = { Text("Review & Import", fontWeight = FontWeight.SemiBold) },
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
                .padding(horizontal = 24.dp)
                .verticalScroll(rememberScrollState()),
            horizontalAlignment = Alignment.CenterHorizontally,
            verticalArrangement = Arrangement.spacedBy(16.dp),
        ) {
            Spacer(modifier = Modifier.height(8.dp))

            Text(
                text = if (isMnemonic) "Received mnemonic (${words.size} words)" else "Received XPRV",
                style = MaterialTheme.typography.titleMedium,
                textAlign = TextAlign.Center,
            )

            Text(
                text = "Review the received secret before importing. Once imported, this will create a new wallet on your device.",
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                textAlign = TextAlign.Center,
            )

            if (isMnemonic) {
                FlowRow(
                    modifier = Modifier
                        .fillMaxWidth()
                        .border(
                            1.dp,
                            MaterialTheme.colorScheme.outline,
                            RoundedCornerShape(12.dp),
                        )
                        .padding(12.dp),
                    horizontalArrangement = Arrangement.spacedBy(8.dp),
                    verticalArrangement = Arrangement.spacedBy(8.dp),
                ) {
                    words.forEachIndexed { index, word ->
                        Row(
                            modifier = Modifier
                                .background(
                                    MaterialTheme.colorScheme.surfaceVariant,
                                    RoundedCornerShape(6.dp),
                                )
                                .padding(horizontal = 8.dp, vertical = 4.dp),
                            horizontalArrangement = Arrangement.spacedBy(4.dp),
                            verticalAlignment = Alignment.CenterVertically,
                        ) {
                            Text(
                                text = "${index + 1}.",
                                fontSize = 12.sp,
                                color = MaterialTheme.colorScheme.onSurfaceVariant,
                            )
                            Text(
                                text = word,
                                fontFamily = FontFamily.Monospace,
                                fontWeight = FontWeight.Medium,
                            )
                        }
                    }
                }
            } else if (payload is KeyTeleportPayload.Xprv) {
                Text(
                    text = payload.xprv,
                    fontFamily = FontFamily.Monospace,
                    fontSize = 11.sp,
                    modifier = Modifier
                        .fillMaxWidth()
                        .border(
                            1.dp,
                            MaterialTheme.colorScheme.outline,
                            RoundedCornerShape(12.dp),
                        )
                        .padding(12.dp),
                )
            }

            Spacer(modifier = Modifier.weight(1f))

            Button(
                onClick = { doImport() },
                enabled = !isImporting,
                modifier = Modifier
                    .fillMaxWidth()
                    .padding(bottom = 24.dp),
            ) {
                Text(if (isImporting) "Importing…" else "Import Wallet")
            }
        }
    }
}
