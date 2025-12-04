package org.bitcoinppl.cove.flow.new_wallet.hot_wallet

import android.util.Log
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.SnackbarHostState
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import org.bitcoinppl.cove.AppManager
import org.bitcoinppl.cove.WalletManager
import org.bitcoinppl.cove.components.FullPageLoadingView
import org.bitcoinppl.cove_core.Route
import org.bitcoinppl.cove_core.WordVerifyStateMachine
import org.bitcoinppl.cove_core.types.WalletId

/**
 * Lifecycle container for verify words flow.
 * Manages WalletManager and WordVerifyStateMachine lifecycle.
 * Shows either VerifyWordsScreen or VerificationCompleteScreen.
 */
@Composable
fun VerifyWordsContainer(
    app: AppManager,
    id: WalletId,
    snackbarHostState: SnackbarHostState = remember { SnackbarHostState() },
) {
    var manager by remember { mutableStateOf<WalletManager?>(null) }
    var stateMachine by remember { mutableStateOf<WordVerifyStateMachine?>(null) }
    var loading by remember { mutableStateOf(true) }
    var verificationComplete by remember { mutableStateOf(false) }
    var showSecretWordsAlert by remember { mutableStateOf(false) }

    LaunchedEffect(id) {
        loading = true
        try {
            val walletManager = app.getWalletManager(id)
            val wordValidator = walletManager.rust.wordValidator()
            val sm = WordVerifyStateMachine(wordValidator, 1u)

            manager = walletManager
            stateMachine = sm

            loading = false
        } catch (e: Exception) {
            Log.e("VerifyWordsContainer", "failed to initialize", e)
            loading = false
        }
    }

    DisposableEffect(Unit) {
        onDispose {
            stateMachine?.destroy()
            stateMachine = null
            manager = null
        }
    }

    when {
        loading -> FullPageLoadingView()
        manager != null && stateMachine != null -> {
            if (verificationComplete) {
                VerificationCompleteScreen(
                    app = app,
                    manager = manager,
                    snackbarHostState = snackbarHostState,
                )
            } else {
                HotWalletVerifyScreen(
                    onBack = { app.popRoute() },
                    onShowWords = { showSecretWordsAlert = true },
                    onSkip = {
                        app.resetRoute(Route.SelectedWallet(id))
                    },
                    stateMachine = stateMachine!!,
                    snackbarHostState = snackbarHostState,
                    onVerificationComplete = {
                        verificationComplete = true
                    },
                )
            }
        }
        else -> {
            FullPageLoadingView()
        }
    }

    if (showSecretWordsAlert) {
        AlertDialog(
            onDismissRequest = { showSecretWordsAlert = false },
            title = { Text("See Secret Words?") },
            text = {
                Text(
                    "Whoever has your secret words has access to your bitcoin. Please keep these safe and don't show them to anyone else.",
                )
            },
            confirmButton = {
                TextButton(
                    onClick = {
                        showSecretWordsAlert = false
                        app.pushRoute(Route.SecretWords(id))
                    },
                ) {
                    Text("Yes, Show Me")
                }
            },
            dismissButton = {
                TextButton(onClick = { showSecretWordsAlert = false }) {
                    Text("Cancel")
                }
            },
        )
    }
}
