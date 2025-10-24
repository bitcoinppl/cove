package org.bitcoinppl.cove.flow.new_wallet.hot_wallet

import android.util.Log
import androidx.compose.material3.SnackbarHostState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import org.bitcoinppl.cove.AppManager
import org.bitcoinppl.cove.WalletManager
import org.bitcoinppl.cove.components.FullPageLoadingView
import org.bitcoinppl.cove_core.*
import org.bitcoinppl.cove_core.types.*

/**
 * lifecycle container for verify words flow
 * manages WalletManager and WordValidator lifecycle
 * shows either VerifyWordsScreen or VerificationCompleteScreen
 */
@Composable
fun VerifyWordsContainer(
    app: AppManager,
    id: WalletId,
    modifier: Modifier = Modifier,
    snackbarHostState: SnackbarHostState = remember { SnackbarHostState() },
) {
    var manager by remember { mutableStateOf<WalletManager?>(null) }
    var validator by remember { mutableStateOf<WordValidator?>(null) }
    var loading by remember { mutableStateOf(true) }
    var verificationComplete by remember { mutableStateOf(false) }

    LaunchedEffect(Unit) {
        try {
            val walletManager = app.getWalletManager(id)
            val wordValidator = walletManager.rust.wordValidator()

            manager = walletManager
            validator = wordValidator
            loading = false
        } catch (e: Exception) {
            Log.e("VerifyWordsContainer", "failed to initialize: $e")
            loading = false
        }
    }

    DisposableEffect(Unit) {
        onDispose {
            // cleanup if needed
        }
    }

    when {
        loading -> FullPageLoadingView()
        manager != null && validator != null -> {
            if (verificationComplete) {
                VerificationCompleteScreen(
                    app = app,
                    manager = manager,
                    snackbarHostState = snackbarHostState,
                )
            } else {
                HotWalletVerifyScreen(
                    onBack = { app.popRoute() },
                    onShowWords = {
                        // navigate to secret words screen
                        // TODO: implement when secret words route is available
                    },
                    onSkip = {
                        // skip verification and go to wallet
                        app.resetRoute(Route.SelectedWallet(id))
                    },
                    validator = validator!!,
                    wordNumber = 1,
                    questionIndex = 1,
                    options = validator!!.possibleWords(1u).map { it.lowercase() },
                    snackbarHostState = snackbarHostState,
                    onCorrectSelected = { word ->
                        // check if verification is complete
                        if (validator!!.isComplete(1u)) {
                            verificationComplete = true
                        }
                    },
                )
            }
        }
        else -> {
            // error state
            FullPageLoadingView()
        }
    }
}
