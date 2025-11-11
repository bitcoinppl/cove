package org.bitcoinppl.cove.tapsigner

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
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Lock
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.launch
import org.bitcoinppl.cove.AppAlertState
import org.bitcoinppl.cove.AppManager
import org.bitcoinppl.cove.TaggedItem
import org.bitcoinppl.cove_core.AfterPinAction
import org.bitcoinppl.cove_core.TapSignerPinAction
import org.bitcoinppl.cove_core.types.Psbt

/**
 * PIN entry screen for TapSigner authentication
 * handles derive, change PIN, backup, and sign actions
 */
@Composable
fun TapSignerEnterPinView(
    app: AppManager,
    manager: TapSignerManager,
    tapSigner: org.bitcoinppl.cove_core.tapcard.TapSigner,
    action: org.bitcoinppl.cove_core.AfterPinAction,
    modifier: Modifier = Modifier,
) {
    var pin by remember { mutableStateOf("") }
    val scope = rememberCoroutineScope()
    val context = LocalContext.current

    val message =
        when (action) {
            is AfterPinAction.Derive ->
                "Enter your TapSigner PIN to import the wallet"
            is AfterPinAction.Change ->
                "Enter your current PIN to change it"
            is AfterPinAction.Backup ->
                "Enter your PIN to backup your TapSigner"
            is AfterPinAction.Sign ->
                "Enter your PIN to sign the transaction"
        }

    // reset pin when screen appears
    LaunchedEffect(Unit) {
        pin = ""
    }

    // launcher for creating backup file
    val createBackupLauncher =
        rememberBackupExportLauncher(app) {
            app.getTapSignerBackup(tapSigner)
                ?: throw IllegalStateException("Backup not available for this TapSigner")
        }

    Column(
        modifier =
            modifier
                .fillMaxSize()
                .verticalScroll(rememberScrollState())
                .padding(horizontal = 16.dp),
        verticalArrangement = Arrangement.spacedBy(40.dp),
    ) {
        // header with cancel button
        Row(
            modifier =
                Modifier
                    .fillMaxWidth()
                    .padding(top = 20.dp),
            horizontalArrangement = Arrangement.Start,
        ) {
            TextButton(onClick = { app.sheetState = null }) {
                Text("Cancel", fontWeight = FontWeight.SemiBold)
            }
        }

        // lock icon
        Icon(
            imageVector = Icons.Default.Lock,
            contentDescription = "Lock",
            modifier = Modifier.size(100.dp).align(Alignment.CenterHorizontally),
            tint = MaterialTheme.colorScheme.primary,
        )

        // title and message
        Column(
            modifier = Modifier.fillMaxWidth().padding(horizontal = 16.dp),
            horizontalAlignment = Alignment.CenterHorizontally,
            verticalArrangement = Arrangement.spacedBy(20.dp),
        ) {
            Text(
                text = "Enter PIN",
                style = MaterialTheme.typography.headlineLarge,
                fontWeight = FontWeight.Bold,
            )

            Text(
                text = message,
                style = MaterialTheme.typography.bodyMedium,
                textAlign = TextAlign.Center,
            )
        }

        // PIN circles
        Box(
            modifier = Modifier.fillMaxWidth(),
            contentAlignment = Alignment.Center,
        ) {
            PinCirclesView(pinLength = pin.length)
        }

        // hidden text field
        HiddenPinTextField(
            value = pin,
            onValueChange = { newPin ->
                pin = newPin
                if (newPin.length == 6) {
                    manager.enteredPin = newPin
                    scope.launch {
                        runAction(app, manager, tapSigner, action, newPin, createBackupLauncher)
                    }
                }
            },
        )

        Spacer(modifier = Modifier.height(40.dp))
    }
}

private suspend fun runAction(
    app: AppManager,
    manager: TapSignerManager,
    tapSigner: org.bitcoinppl.cove_core.tapcard.TapSigner,
    action: org.bitcoinppl.cove_core.AfterPinAction,
    pin: String,
    createBackupLauncher: androidx.activity.result.ActivityResultLauncher<String>,
) {
    val nfc = manager.getOrCreateNfc(tapSigner)

    when (action) {
        is AfterPinAction.Derive -> {
            deriveAction(app, manager, nfc, tapSigner, pin)
        }
        is AfterPinAction.Change -> {
            changeAction(manager, tapSigner, pin)
        }
        is AfterPinAction.Backup -> {
            backupAction(app, nfc, tapSigner, pin, createBackupLauncher)
        }
        is AfterPinAction.Sign -> {
            signAction(app, nfc, action.v1, pin)
        }
    }
}

private suspend fun deriveAction(
    app: AppManager,
    manager: TapSignerManager,
    nfc: TapSignerNfcHelper,
    tapSigner: org.bitcoinppl.cove_core.tapcard.TapSigner,
    pin: String,
) {
    try {
        val deriveInfo = nfc.derive(pin)
        manager.resetRoute(
            org.bitcoinppl.cove_core.TapSignerRoute.ImportSuccess(
                tapSigner,
                deriveInfo,
            ),
        )
    } catch (e: Exception) {
        // handle auth errors silently, show alert for other errors
        if (!isAuthError(e)) {
            app.alertState =
                org.bitcoinppl.cove.TaggedItem(
                    org.bitcoinppl.cove.AppAlertState.TapSignerDeriveFailed(
                        "Failed to derive wallet: ${e.message ?: "Unknown error occurred"}",
                    ),
                )
        }
    }
}

private fun changeAction(
    manager: TapSignerManager,
    tapSigner: org.bitcoinppl.cove_core.tapcard.TapSigner,
    pin: String,
) {
    manager.navigate(
        org.bitcoinppl.cove_core.TapSignerRoute.NewPin(
            org.bitcoinppl.cove_core.TapSignerNewPinArgs(
                tapSigner = tapSigner,
                startingPin = pin,
                chainCode = null,
                action = TapSignerPinAction.CHANGE,
            ),
        ),
    )
}

private suspend fun backupAction(
    app: AppManager,
    nfc: TapSignerNfcHelper,
    tapSigner: org.bitcoinppl.cove_core.tapcard.TapSigner,
    pin: String,
    createBackupLauncher: androidx.activity.result.ActivityResultLauncher<String>,
) {
    try {
        val backup = nfc.backup(pin)
        // save backup and show export dialog
        app.saveTapSignerBackup(tapSigner, backup)

        // trigger backup export
        val fileName = "${tapSigner.identFileNamePrefix()}_backup.txt"
        createBackupLauncher.launch(fileName)
    } catch (e: Exception) {
        if (!isAuthError(e)) {
            app.alertState =
                org.bitcoinppl.cove.TaggedItem(
                    org.bitcoinppl.cove.AppAlertState.General(
                        title = "Backup Failed!",
                        message = "Failed to create backup: ${e.message ?: "Unknown error occurred"}",
                    ),
                )
        }
    }
}

private suspend fun signAction(
    app: AppManager,
    nfc: TapSignerNfcHelper,
    psbt: Psbt,
    pin: String,
) {
    try {
        val signedPsbt = nfc.sign(psbt, pin)
        val db = org.bitcoinppl.cove_core.Database().unsignedTransactions()
        val txId = psbt.txId()
        val record = db.getTxThrow(txId = txId)
        val route =
            org.bitcoinppl.cove_core.RouteFactory().sendConfirm(
                id = record.walletId(),
                details = record.confirmDetails(),
                signedPsbt = signedPsbt,
            )

        app.sheetState = null
        app.pushRoute(route)
    } catch (e: Exception) {
        if (!isAuthError(e)) {
            app.alertState =
                org.bitcoinppl.cove.TaggedItem(
                    org.bitcoinppl.cove.AppAlertState.General(
                        title = "Signing Failed!",
                        message = "Failed to sign transaction: ${e.message ?: "Unknown error occurred"}",
                    ),
                )
            app.sheetState = null
        }
    }
}

private fun isAuthError(error: Exception): Boolean {
    // check if error is a bad auth error
    return error.message?.contains("BadAuth", ignoreCase = true) == true
}
