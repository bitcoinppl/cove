package org.bitcoinppl.cove

import androidx.compose.material3.SnackbarHostState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import org.bitcoinppl.cove.components.FullPageLoadingView
import org.bitcoinppl.cove.flow.new_wallet.hot_wallet.HotWalletCreateScreen
import org.bitcoinppl.cove.flow.new_wallet.hot_wallet.HotWalletImportScreen
import org.bitcoinppl.cove.flow.new_wallet.hot_wallet.HotWalletSelectScreen
import org.bitcoinppl.cove.flow.new_wallet.hot_wallet.VerifyWordsContainer
import org.bitcoinppl.cove_core.*
import org.bitcoinppl.cove_core.types.*

/**
 * new hot wallet container - routes to hot wallet flow screens
 * ported from iOS NewHotWalletContainer.swift
 */
@Composable
fun NewHotWalletContainer(
    app: AppManager,
    route: HotWalletRoute,
    modifier: Modifier = Modifier,
) {
    val snackbarHostState = remember { SnackbarHostState() }

    when (route) {
        is HotWalletRoute.Select -> {
            HotWalletSelectScreen(
                app = app,
                snackbarHostState = snackbarHostState,
            )
        }
        is HotWalletRoute.Create -> {
            // lifecycle container for PendingWalletManager
            PendingWalletContainer(
                app = app,
                numberOfWords = route.v1,
                snackbarHostState = snackbarHostState,
            )
        }
        is HotWalletRoute.Import -> {
            // lifecycle container for ImportWalletManager
            ImportWalletContainer(
                app = app,
                numberOfWords = route.v1,
                importType = route.v2,
                snackbarHostState = snackbarHostState,
            )
        }
        is HotWalletRoute.VerifyWords -> {
            VerifyWordsContainer(
                app = app,
                id = route.v1,
                snackbarHostState = snackbarHostState,
            )
        }
    }
}

/**
 * lifecycle container for pending wallet creation flow
 */
@Composable
private fun PendingWalletContainer(
    app: AppManager,
    numberOfWords: NumberOfBip39Words,
    snackbarHostState: SnackbarHostState,
) {
    var manager by remember { mutableStateOf<PendingWalletManager?>(null) }
    var loading by remember { mutableStateOf(true) }

    LaunchedEffect(Unit) {
        manager = PendingWalletManager(numberOfWords)
        loading = false
    }

    DisposableEffect(Unit) {
        onDispose {
            // cleanup if needed
        }
    }

    when {
        loading -> FullPageLoadingView()
        manager != null ->
            HotWalletCreateScreen(
                app = app,
                manager = manager!!,
                snackbarHostState = snackbarHostState,
            )
        else -> FullPageLoadingView()
    }
}

/**
 * lifecycle container for import wallet flow
 */
@Composable
private fun ImportWalletContainer(
    app: AppManager,
    numberOfWords: NumberOfBip39Words,
    importType: ImportType,
    snackbarHostState: SnackbarHostState,
) {
    var manager by remember { mutableStateOf<ImportWalletManager?>(null) }
    var loading by remember { mutableStateOf(true) }

    LaunchedEffect(Unit) {
        manager = ImportWalletManager()
        loading = false
    }

    DisposableEffect(Unit) {
        onDispose {
            // cleanup if needed
        }
    }

    when {
        loading -> FullPageLoadingView()
        manager != null ->
            HotWalletImportScreen(
                app = app,
                manager = manager!!,
                numberOfWords = numberOfWords,
                importType = importType,
                snackbarHostState = snackbarHostState,
            )
        else -> FullPageLoadingView()
    }
}
