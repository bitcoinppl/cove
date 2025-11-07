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
import androidx.compose.runtime.mutableIntStateOf
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
    var showSecretWordsAlert by remember { mutableStateOf(false) }

    // verification state - tracks current word being verified
    var wordNumber by remember { mutableIntStateOf(1) }
    var questionIndex by remember { mutableIntStateOf(1) }
    var possibleWords by remember { mutableStateOf<List<String>>(emptyList()) }

    LaunchedEffect(id) {
        loading = true
        try {
            val walletManager = app.getWalletManager(id)
            val wordValidator = walletManager.rust.wordValidator()

            manager = walletManager
            validator = wordValidator

            // initialize possible words for first question
            possibleWords = wordValidator.possibleWords(1u.toUByte()).map { it.lowercase() }

            loading = false
        } catch (e: Exception) {
            Log.e("VerifyWordsContainer", "failed to initialize", e)
            loading = false
        }
    }

    DisposableEffect(Unit) {
        onDispose {
            // clean up validator to prevent resource leak
            validator?.destroy()
            validator = null
            manager = null
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
                    onShowWords = { showSecretWordsAlert = true },
                    onSkip = {
                        // skip verification and go to wallet
                        app.resetRoute(Route.SelectedWallet(id))
                    },
                    validator = validator!!,
                    wordNumber = wordNumber,
                    questionIndex = questionIndex,
                    options = possibleWords,
                    snackbarHostState = snackbarHostState,
                    onCorrectSelected = { word ->
                        // check if the word is correct
                        if (!validator!!.isWordCorrect(word, wordNumber.toUByte())) {
                            return@HotWalletVerifyScreen
                        }

                        // check if verification is complete
                        if (validator!!.isComplete(wordNumber.toUByte())) {
                            verificationComplete = true
                            return@HotWalletVerifyScreen
                        }

                        // advance to next word
                        wordNumber += 1
                        questionIndex += 1
                        possibleWords = validator!!.possibleWords(wordNumber.toUByte()).map { it.lowercase() }
                    },
                )
            }
        }
        else -> {
            // error state
            FullPageLoadingView()
        }
    }

    // secret words confirmation dialog
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
