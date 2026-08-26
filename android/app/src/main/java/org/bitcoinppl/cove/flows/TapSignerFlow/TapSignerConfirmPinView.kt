package org.bitcoinppl.cove.flows.TapSignerFlow

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
import androidx.compose.material.icons.automirrored.filled.ArrowBack
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
import org.bitcoinppl.cove.TaggedItem
import org.bitcoinppl.cove.runCatchingCancellable
import org.bitcoinppl.cove_core.AfterPinAction
import org.bitcoinppl.cove_core.AppAlertState
import org.bitcoinppl.cove_core.SetupCmdResponse
import org.bitcoinppl.cove_core.TapSignerConfirmPinArgs
import org.bitcoinppl.cove_core.TapSignerPinAction
import org.bitcoinppl.cove_core.TapSignerRoute

/** Confirm a new CVC and run setup or CVC change. */
@Composable
fun TapSignerConfirmPinView(
    app: AppManager,
    manager: TapSignerManager,
    args: TapSignerConfirmPinArgs,
    modifier: Modifier = Modifier,
) {
    var confirmCvc by remember { mutableStateOf("") }
    var validationError by remember { mutableStateOf<String?>(null) }
    var isSubmitting by remember { mutableStateOf(false) }
    val scope = rememberCoroutineScope()

    DisposableEffect(Unit) {
        onDispose { confirmCvc = "" }
    }

    fun submit() {
        if (isSubmitting || !isValidCvc(confirmCvc)) return

        if (confirmCvc != args.newPin) {
            confirmCvc = ""
            validationError = "The CVCs do not match"
            return
        }

        validationError = null
        isSubmitting = true
        scope.launch {
            try {
                when (args.action) {
                    TapSignerPinAction.SETUP -> setupTapSigner(app, manager, args)
                    TapSignerPinAction.CHANGE -> changeTapSignerPin(app, manager, args)
                }
            } finally {
                manager.endScan()
                confirmCvc = ""
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
            TextButton(onClick = { manager.popRoute() }) {
                Icon(
                    imageVector = Icons.AutoMirrored.Filled.ArrowBack,
                    contentDescription = "Back",
                )
                Text("Back", fontWeight = FontWeight.SemiBold)
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
                text = "Confirm New CVC",
                style = MaterialTheme.typography.headlineLarge,
                fontWeight = FontWeight.Bold,
            )

            Text(
                text =
                    "Confirm the same 6–32 ASCII digits.",
                style = MaterialTheme.typography.bodyMedium,
                textAlign = TextAlign.Center,
            )

            TapSignerCvcInput(
                value = confirmCvc,
                onValueChange = {
                    confirmCvc = it
                    if (validationError != null) validationError = null
                },
                label = "Confirm CVC",
                options =
                    TapSignerCvcInputOptions(
                        testTag = "tapSignerConfirm.newCvc",
                        validationError = validationError,
                    ),
            )
        }

        Button(
            onClick = ::submit,
            enabled = !isSubmitting && isValidCvc(confirmCvc),
            modifier = Modifier.fillMaxWidth().testTag("tapSignerConfirm.submit"),
        ) {
            Text(if (isSubmitting) "Working…" else "Confirm and continue")
        }

        Spacer(modifier = Modifier.height(20.dp))
    }
}

private suspend fun setupTapSigner(
    app: AppManager,
    manager: TapSignerManager,
    args: TapSignerConfirmPinArgs,
) {
    val chainCodeBytes =
        args.chainCode?.let { chainCode ->
            decodeChainCode(chainCode)
                ?: run {
                    app.alertState =
                        TaggedItem(
                            AppAlertState.General(
                                title = "Invalid chain code",
                                message = "Chain code must be exactly 64 hexadecimal characters (32 bytes)",
                            ),
                        )
                    return
                }
        }

    val nfc = manager.getOrCreateNfc(args.tapSigner)
    manager.beginScan("Hold your phone near the TapSigner to set up")

    val result =
        runCatchingCancellable("TapSignerConfirmPin", "TapSigner setup failed") {
            nfc.setupTapSigner(
                factoryCvc = args.startingPin,
                newCvc = args.newPin,
                chainCode = chainCodeBytes,
                callbacks = manager.operationCallbacks(),
            )
        }
    val response = result.getOrNull()

    if (response == null) {
        val setupResponse = nfc.lastSetupResponse()
        if (setupResponse != null) {
            manager.resetRoute(TapSignerRoute.SetupRetry(args.tapSigner, setupResponse))
        } else {
            app.sheetState = null
            app.alertState =
                TaggedItem(
                    AppAlertState.TapSignerSetupFailed(
                        "TapSigner setup failed. Please try again.",
                    ),
                )
        }
        return
    }

    when (response) {
        is SetupCmdResponse.Complete -> {
            manager.resetRoute(TapSignerRoute.SetupSuccess(args.tapSigner, response.v1))
        }
        is SetupCmdResponse.Retry -> {
            manager.resetRoute(TapSignerRoute.SetupRetry(args.tapSigner, response))
        }
    }
}

private suspend fun changeTapSignerPin(
    app: AppManager,
    manager: TapSignerManager,
    args: TapSignerConfirmPinArgs,
) {
    val nfc = manager.getOrCreateNfc(args.tapSigner)
    manager.beginScan("Hold your phone near the TapSigner to change its CVC")

    val result =
        runCatchingCancellable("TapSignerConfirmPin", "TapSigner CVC change failed") {
        nfc.changePin(
            currentCvc = args.startingPin,
            newCvc = args.newPin,
            callbacks = manager.operationCallbacks(),
        )
        }
    val error = result.exceptionOrNull()

    if (error == null) {
        app.sheetState = null
        app.alertState =
            TaggedItem(
                AppAlertState.General(
                    title = "CVC Changed",
                    message = "Your TAPSIGNER CVC was changed successfully.",
                ),
            )
        return
    }

    when {
        error is TapSignerOperationRetryException -> {
            manager.errorMessage = "The CVC change needs another scan of the same card. Please try again"
        }
        isAuthError(error) -> {
            app.sheetState = null
            app.alertState =
                TaggedItem(
                    AppAlertState.TapSignerWrongPin(
                        args.tapSigner,
                        AfterPinAction.Change,
                    ),
                )
        }
        isNoBackupError(error) -> {
            app.alertState = TaggedItem(AppAlertState.TapSignerNoBackup(args.tapSigner))
        }
        else -> {
            app.alertState =
                TaggedItem(
                    AppAlertState.General(
                        title = "Error",
                        message = "TapSigner CVC change failed. Please try again.",
                    ),
                )
        }
    }
}
