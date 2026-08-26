package org.bitcoinppl.cove.flows.TapSignerFlow

import androidx.activity.result.ActivityResultLauncher
import androidx.compose.foundation.layout.Arrangement
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
import androidx.compose.material3.Button
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.launch
import org.bitcoinppl.cove.AppManager
import org.bitcoinppl.cove.Log
import org.bitcoinppl.cove.TaggedItem
import org.bitcoinppl.cove.runCatchingCancellable
import org.bitcoinppl.cove_core.AfterPinAction
import org.bitcoinppl.cove_core.AppAlertState
import org.bitcoinppl.cove_core.TapSignerConfirmPinArgs
import org.bitcoinppl.cove_core.TapSignerNewPinArgs
import org.bitcoinppl.cove_core.TapSignerPinAction
import org.bitcoinppl.cove_core.TapSignerRoute
import org.bitcoinppl.cove_core.types.Psbt

/** Enter a TAPSIGNER CVC before a card operation. */
@Composable
fun TapSignerEnterPinView(
    app: AppManager,
    manager: TapSignerManager,
    tapSigner: org.bitcoinppl.cove_core.tapcard.TapSigner,
    action: AfterPinAction,
    modifier: Modifier = Modifier,
) {
    var cvc by remember { mutableStateOf("") }
    var isSubmitting by remember { mutableStateOf(false) }
    val scope = rememberCoroutineScope()
    val message = action.userMessage()
    val createBackupLauncher =
        rememberBackupExportLauncher(app) {
            app.getTapSignerBackup(tapSigner)
                ?: error("Backup is not available for this TAPSIGNER")
        }

    DisposableEffect(Unit) {
        onDispose {
            cvc = ""
        }
    }

    fun submit() {
        if (isSubmitting || !isValidCvc(cvc)) return

        isSubmitting = true
        scope.launch {
            try {
                runAction(
                    app = app,
                    manager = manager,
                    tapSigner = tapSigner,
                    action = action,
                    cvc = cvc,
                    createBackupLauncher = createBackupLauncher,
                )
            } finally {
                cvc = ""
                isSubmitting = false
            }
        }
    }

    Column(
        modifier =
            modifier
                .fillMaxSize()
                .verticalScroll(rememberScrollState())
                .padding(horizontal = 16.dp),
        verticalArrangement = Arrangement.spacedBy(32.dp),
    ) {
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

        Icon(
            imageVector = Icons.Default.Lock,
            contentDescription = "Lock",
            modifier = Modifier.size(100.dp).align(Alignment.CenterHorizontally),
            tint = MaterialTheme.colorScheme.primary,
        )

        Column(
            modifier = Modifier.fillMaxWidth().padding(horizontal = 16.dp),
            horizontalAlignment = Alignment.CenterHorizontally,
            verticalArrangement = Arrangement.spacedBy(20.dp),
        ) {
            Text(
                text = "Enter TAPSIGNER CVC",
                style = MaterialTheme.typography.headlineLarge,
                fontWeight = FontWeight.Bold,
            )

            Text(
                text = message,
                style = MaterialTheme.typography.bodyMedium,
                textAlign = TextAlign.Center,
            )

            TapSignerCvcInput(
                value = cvc,
                onValueChange = { cvc = it },
                label = "CVC",
                options = TapSignerCvcInputOptions(testTag = "tapSignerEnter.cvc"),
            )
        }

        Button(
            onClick = ::submit,
            enabled = !isSubmitting && isValidCvc(cvc),
            modifier = Modifier.fillMaxWidth().testTag("tapSignerEnter.submit"),
        ) {
            Text(if (isSubmitting) "Working…" else "Submit")
        }

        Spacer(modifier = Modifier.height(20.dp))
    }
}

private suspend fun runAction(
    app: AppManager,
    manager: TapSignerManager,
    tapSigner: org.bitcoinppl.cove_core.tapcard.TapSigner,
    action: AfterPinAction,
    cvc: String,
    createBackupLauncher: ActivityResultLauncher<String>,
) {
    when (action) {
        AfterPinAction.Derive -> deriveAction(app, manager, tapSigner, cvc)
        AfterPinAction.Change -> changeAction(manager, tapSigner, cvc)
        AfterPinAction.Backup -> {
            backupAction(app, manager, tapSigner, cvc, createBackupLauncher)
        }
        is AfterPinAction.Sign -> signAction(app, manager, tapSigner, action.v1, cvc)
    }
}

private suspend fun deriveAction(
    app: AppManager,
    manager: TapSignerManager,
    tapSigner: org.bitcoinppl.cove_core.tapcard.TapSigner,
    cvc: String,
) {
    val nfc = manager.getOrCreateNfc(tapSigner)
    manager.beginScan("Hold your phone near the TapSigner to import the wallet")
    val result = try {
        runCatchingCancellable("TapSignerEnterPin", "TapSigner import failed") {
            nfc.derive(cvc, manager.operationCallbacks())
        }
    } finally {
        manager.endScan()
    }
    val deriveInfo = result.getOrNull()
    val error = result.exceptionOrNull()

    when {
        deriveInfo != null -> manager.resetRoute(TapSignerRoute.ImportSuccess(tapSigner, deriveInfo))
        error is TapSignerOperationRetryException -> {
            manager.resetRoute(TapSignerRoute.ImportRetry(tapSigner))
        }
        error != null && isAuthError(error) -> {
            app.sheetState = null
            app.alertState =
                TaggedItem(
                    AppAlertState.TapSignerWrongPin(
                        tapSigner,
                        AfterPinAction.Derive,
                    ),
                )
        }
        error != null -> {
            Log.w("TapSignerEnterPin", "TapSigner import failed")
            manager.errorMessage = "Connection failed, please try again"
        }
    }
}

private fun changeAction(
    manager: TapSignerManager,
    tapSigner: org.bitcoinppl.cove_core.tapcard.TapSigner,
    cvc: String,
) {
    manager.navigate(
        TapSignerRoute.NewPin(
            TapSignerNewPinArgs(
                tapSigner = tapSigner,
                startingPin = cvc,
                chainCode = null,
                action = TapSignerPinAction.CHANGE,
            ),
        ),
    )
}

private suspend fun backupAction(
    app: AppManager,
    manager: TapSignerManager,
    tapSigner: org.bitcoinppl.cove_core.tapcard.TapSigner,
    cvc: String,
    createBackupLauncher: ActivityResultLauncher<String>,
) {
    val nfc = manager.getOrCreateNfc(tapSigner)
    manager.beginScan("Hold your phone near the TapSigner to back up the card")

    val result = try {
        runCatchingCancellable("TapSignerEnterPin", "TapSigner backup failed") {
            nfc.backup(cvc, manager.operationCallbacks())
        }
    } finally {
        manager.endScan()
    }
    val backup = result.getOrNull()
    val error = result.exceptionOrNull()

    if (backup != null) {
        app.saveTapSignerBackup(tapSigner, backup)
        createBackupLauncher.launch("${tapSigner.identFileNamePrefix()}_backup.txt")
        app.sheetState = null
        return
    }

    when {
        error is TapSignerOperationRetryException -> {
        manager.errorMessage = "The backup needs another card scan. Please try again"
        }
        error != null && isAuthError(error) -> {
            app.sheetState = null
            app.alertState =
                TaggedItem(AppAlertState.TapSignerWrongPin(tapSigner, AfterPinAction.Backup))
        }
        error != null -> {
            manager.errorMessage = "Connection failed, please try again"
        }
    }
}

private suspend fun signAction(
    app: AppManager,
    manager: TapSignerManager,
    tapSigner: org.bitcoinppl.cove_core.tapcard.TapSigner,
    psbt: Psbt,
    cvc: String,
) {
    val nfc = manager.getOrCreateNfc(tapSigner)
    manager.beginScan("Hold your phone near the TapSigner to sign")

    val result = try {
        runCatchingCancellable("TapSignerEnterPin", "TapSigner signing failed") {
            nfc.sign(psbt, cvc, manager.operationCallbacks())
        }
    } finally {
        manager.endScan()
    }
    val signedPsbt = result.getOrNull()
    val error = result.exceptionOrNull()

    if (signedPsbt != null) {
        val route =
            try {
                runCatchingCancellable("TapSignerEnterPin", "Failed to open signed PSBT") {
                    val db = org.bitcoinppl.cove_core.Database().unsignedTransactions()
                    val record = db.getTxThrow(psbt.txId())
                    org.bitcoinppl.cove_core.RouteFactory().use { factory ->
                        factory.sendConfirmSignedPsbt(
                            id = record.walletId(),
                            details = record.confirmDetails(),
                            psbt = signedPsbt,
                        )
                    }
                }.getOrNull()
            } finally {
                signedPsbt.destroy()
            }

        if (route != null) {
            app.sheetState = null
            app.pushRoute(route)
        } else {
            manager.errorMessage = "Signing completed, but the transaction could not be opened"
        }

        return
    }

    when {
        error is TapSignerOperationRetryException -> {
        manager.errorMessage = "Signing needs another card scan. Please try again"
        }
        error != null && isAuthError(error) -> {
            app.sheetState = null
            app.alertState =
                TaggedItem(
                    AppAlertState.TapSignerWrongPin(
                        tapSigner,
                        AfterPinAction.Sign(psbt),
                    ),
                )
        }
        error != null -> {
            manager.errorMessage = "Connection failed, please try again"
        }
    }
}
