package org.bitcoinppl.cove.secret_words

import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.Send
import androidx.compose.material.icons.filled.MoreVert
import androidx.compose.material.icons.filled.QrCode
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
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
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.LifecycleEventObserver
import androidx.lifecycle.compose.LocalLifecycleOwner
import org.bitcoinppl.cove.AppManager
import org.bitcoinppl.cove.AuthManager
import org.bitcoinppl.cove.QrCodeGenerator
import org.bitcoinppl.cove.TaggedItem
import org.bitcoinppl.cove.flows.SettingsFlow.hasFreshMainCredential
import org.bitcoinppl.cove.views.ColumnMajorGrid
import org.bitcoinppl.cove.views.RecoveryWordChip
import org.bitcoinppl.cove_core.AppAlertState
import org.bitcoinppl.cove_core.types.WalletId

internal enum class SecretWordsSensitiveAction(
    val confirmationTitle: String,
    val confirmationButtonTitle: String,
    val confirmationMessage: String,
) {
    SEED_QR(
        confirmationTitle = "Show Seed QR?",
        confirmationButtonTitle = "Show QR Code",
        confirmationMessage =
            "Your seed words are sensitive and control access to your Bitcoin. " +
                "QR codes are machine-readable, so be careful who or what device you show this to.",
    ),
    KEY_TELEPORT(
        confirmationTitle = "Send with KeyTeleport?",
        confirmationButtonTitle = "Continue",
        confirmationMessage =
            "KeyTeleport sends this wallet's secret words to another device. " +
                "Only continue if you trust the receiving device and can verify its request.",
    ),
    ;

    fun perform(
        showSeedQr: () -> Unit,
        startKeyTeleport: () -> Unit,
    ) {
        when (this) {
            SEED_QR -> showSeedQr()
            KEY_TELEPORT -> startKeyTeleport()
        }
    }
}

/**
 * Build the KeyTeleport send request for this wallet's secret words.
 *
 * The send is held until the main credential is entered again, so neither a decoy session nor an
 * unlock that happened before the request can start a transfer.
 */
@Composable
internal fun rememberKeyTeleportSendRequest(
    app: AppManager,
    auth: AuthManager,
    walletId: WalletId,
): () -> Unit {
    var pendingGeneration by remember(walletId) { mutableStateOf<Long?>(null) }
    val lifecycleOwner = LocalLifecycleOwner.current

    LaunchedEffect(auth.mainCredentialGeneration) {
        val requestedAt = pendingGeneration ?: return@LaunchedEffect
        if (!hasFreshMainCredential(auth.mainCredentialGeneration, requestedAt)) return@LaunchedEffect

        pendingGeneration = null
        if (!app.startKeyTeleportSend(walletId)) {
            app.alertState =
                TaggedItem(
                    AppAlertState.General(
                        title = "KeyTeleport",
                        message = "Finish the active KeyTeleport flow before starting another transfer.",
                    ),
                )
        }
    }

    DisposableEffect(lifecycleOwner) {
        val observer =
            LifecycleEventObserver { _, event ->
                if (event == Lifecycle.Event.ON_PAUSE || event == Lifecycle.Event.ON_STOP) {
                    pendingGeneration = null
                }
            }
        lifecycleOwner.lifecycle.addObserver(observer)

        onDispose {
            lifecycleOwner.lifecycle.removeObserver(observer)
            pendingGeneration = null
        }
    }

    return remember(app, auth, walletId) {
        {
            if (!auth.isAuthEnabled) {
                app.alertState =
                    TaggedItem(
                        AppAlertState.General(
                            title = "KeyTeleport",
                            message = "Set up a PIN or biometric unlock before sending secret words with KeyTeleport.",
                        ),
                    )
            } else {
                pendingGeneration = auth.mainCredentialGeneration
                auth.lock()
            }
        }
    }
}

@Composable
internal fun SecretWordsToolbarMenu(
    showKeyTeleport: Boolean,
    onAction: (SecretWordsSensitiveAction) -> Unit,
) {
    var isExpanded by remember { mutableStateOf(false) }

    IconButton(onClick = { isExpanded = true }) {
        Icon(
            imageVector = Icons.Default.MoreVert,
            contentDescription = "Secret words options",
            tint = Color.White,
        )
    }
    DropdownMenu(
        expanded = isExpanded,
        onDismissRequest = { isExpanded = false },
    ) {
        DropdownMenuItem(
            text = { Text("Seed QR") },
            leadingIcon = {
                Icon(Icons.Default.QrCode, contentDescription = null)
            },
            onClick = {
                isExpanded = false
                onAction(SecretWordsSensitiveAction.SEED_QR)
            },
        )
        if (showKeyTeleport) {
            DropdownMenuItem(
                text = { Text("KeyTeleport") },
                leadingIcon = {
                    Icon(Icons.AutoMirrored.Filled.Send, contentDescription = null)
                },
                onClick = {
                    isExpanded = false
                    onAction(SecretWordsSensitiveAction.KEY_TELEPORT)
                },
            )
        }
    }
}

@Composable
internal fun SecretWordsActionConfirmation(
    action: SecretWordsSensitiveAction,
    onConfirm: () -> Unit,
    onDismiss: () -> Unit,
) {
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text(action.confirmationTitle) },
        text = { Text(action.confirmationMessage) },
        confirmButton = {
            TextButton(onClick = onConfirm) {
                Text(action.confirmationButtonTitle)
            }
        },
        dismissButton = {
            TextButton(onClick = onDismiss) {
                Text("Cancel")
            }
        },
    )
}

/**
 * Recovery words grid for viewing only (non-selectable)
 * Uses column-major ordering (words flow down columns first)
 */
@Composable
internal fun RecoveryWordsGrid(
    words: List<String>,
    modifier: Modifier = Modifier,
) {
    ColumnMajorGrid(
        items = words,
        modifier = modifier,
        numColumns = 2,
    ) { index, word ->
        RecoveryWordChip(
            index = index + 1,
            word = word,
            selected = false,
            onClick = null,
        )
    }
}

@Composable
internal fun SeedQrSheetContent(seedQrString: String) {
    Column(
        modifier =
            Modifier
                .fillMaxWidth()
                .padding(horizontal = 16.dp)
                .padding(bottom = 32.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        Text(
            text = "Seed QR",
            style = MaterialTheme.typography.titleLarge,
            fontWeight = FontWeight.SemiBold,
        )

        Spacer(modifier = Modifier.height(8.dp))

        Text(
            text = "Scan with a SeedQR-compatible device",
            style = MaterialTheme.typography.bodyMedium,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            textAlign = TextAlign.Center,
        )

        Spacer(modifier = Modifier.height(24.dp))

        BoxWithConstraints(
            modifier =
                Modifier
                    .fillMaxWidth()
                    .padding(horizontal = 16.dp),
            contentAlignment = Alignment.Center,
        ) {
            val qrSize = maxWidth
            val bitmap =
                remember(seedQrString) {
                    QrCodeGenerator.generate(seedQrString, size = 512)
                }

            Box(
                modifier =
                    Modifier
                        .size(qrSize)
                        .clip(RoundedCornerShape(12.dp))
                        .background(Color.White)
                        .padding(12.dp),
                contentAlignment = Alignment.Center,
            ) {
                Image(
                    bitmap = bitmap.asImageBitmap(),
                    contentDescription = "Seed QR Code",
                    modifier = Modifier.fillMaxSize(),
                    contentScale = ContentScale.FillBounds,
                )
            }
        }

        Spacer(modifier = Modifier.height(32.dp))
    }
}
