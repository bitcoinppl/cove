package org.bitcoinppl.cove.flows.SettingsFlow

import androidx.activity.compose.BackHandler
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.Button
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.delay
import kotlinx.coroutines.ensureActive
import org.bitcoinppl.cove.AppManager
import org.bitcoinppl.cove.R
import org.bitcoinppl.cove.TaggedItem
import org.bitcoinppl.cove.WalletManager
import org.bitcoinppl.cove.WalletSelectionRecoveryResult
import org.bitcoinppl.cove.recoverWalletSelectionOrPopRoute
import org.bitcoinppl.cove_core.*
import org.bitcoinppl.cove_core.AppAlertState
import org.bitcoinppl.cove_core.types.*
import kotlin.coroutines.cancellation.CancellationException

/**
 * Wallet settings container - lazy loads WalletManager for wallet settings
 * Ported from iOS WalletSettingsContainer.swift
 */
@Composable
fun WalletSettingsContainer(
    app: AppManager,
    id: WalletId,
    route: WalletSettingsRoute,
    modifier: Modifier = Modifier,
) {
    var loadState by remember(id) {
        mutableStateOf<WalletSettingsLoadState>(WalletSettingsLoadState.Loading)
    }
    var loadAttempt by remember(id) { mutableStateOf(0) }
    var recoveryGeneration by remember(id) { mutableStateOf(0) }
    var lastLoadFailureAlertMessage by remember(id, route) { mutableStateOf<String?>(null) }
    val tag = "WalletSettingsContainer"
    val loadErrorTitle = stringResource(R.string.settings_wallet_load_error_title)
    val unknownError = stringResource(R.string.common_remaining_unknown_error)
    val loadErrorMessageWithDetail = stringResource(R.string.settings_wallet_load_error_message_with_detail)

    fun startWalletSelectionRecovery(message: String) {
        if (loadState is WalletSettingsLoadState.Recovering && !app.isNavigationSettled) return

        recoveryGeneration += 1
        loadState = WalletSettingsLoadState.Recovering(message)

        when (
            val result =
                recoverWalletSelectionOrPopRoute(
                    selectLatestOrNewWallet = app::selectLatestOrNewWallet,
                    popRoute = app::popRouteForRecovery,
                )
        ) {
            WalletSelectionRecoveryResult.Recovered -> {
                loadState = WalletSettingsLoadState.Loading
            }
            is WalletSelectionRecoveryResult.PoppedRoute -> {
                android.util.Log.e(tag, "failed to recover wallet selection", result.recoveryError)
            }
            is WalletSelectionRecoveryResult.NoRouteToPop -> {
                android.util.Log.e(tag, "failed to recover wallet selection", result.recoveryError)
                android.util.Log.e(tag, "no route available to leave wallet settings after recovery failure")
                loadState = WalletSettingsLoadState.Failed(message)
            }
            is WalletSelectionRecoveryResult.FailedToPopRoute -> {
                android.util.Log.e(tag, "failed to recover wallet selection", result.recoveryError)
                android.util.Log.e(tag, "failed to leave wallet settings after recovery failure", result.navigationError)
                loadState = WalletSettingsLoadState.Failed(message)
            }
        }
    }

    LaunchedEffect(loadState, app.isNavigationSettled) {
        val state = loadState
        if (state is WalletSettingsLoadState.Recovering && app.isNavigationSettled) {
            loadState = WalletSettingsLoadState.Failed(state.message)
        }
    }

    // lazy load wallet manager
    LaunchedEffect(id, loadAttempt) {
        loadState = WalletSettingsLoadState.Loading

        try {
            android.util.Log.d(tag, "getting wallet $id")
            loadState = WalletSettingsLoadState.Ready(app.getWalletManager(id))
        } catch (e: CancellationException) {
            throw e
        } catch (e: Exception) {
            val message = e.message ?: unknownError

            android.util.Log.e(tag, "failed to load wallet", e)
            recoveryGeneration += 1
            val failureGeneration = recoveryGeneration
            loadState = WalletSettingsLoadState.Failed(message)

            if (lastLoadFailureAlertMessage != message) {
                app.alertState =
                    TaggedItem(
                        AppAlertState.General(
                            title = loadErrorTitle,
                            message = loadErrorMessageWithDetail.format(message),
                        ),
                    )
                lastLoadFailureAlertMessage = message
            }

            // leave the alert visible before route recovery replaces this screen
            delay(WALLET_LOAD_ERROR_RECOVERY_DELAY_MS)
            ensureActive()

            val state = loadState
            if (
                recoveryGeneration == failureGeneration &&
                state is WalletSettingsLoadState.Failed &&
                state.message == message
            ) {
                startWalletSelectionRecovery(message)
            }
        }
    }

    // render
    when (val state = loadState) {
        WalletSettingsLoadState.Loading,
        is WalletSettingsLoadState.Recovering -> {
            Box(
                modifier = modifier.fillMaxSize(),
                contentAlignment = Alignment.Center,
            ) {
                CircularProgressIndicator()
            }
        }

        is WalletSettingsLoadState.Failed -> {
            val recoverWalletSelection = { startWalletSelectionRecovery(state.message) }

            BackHandler(onBack = recoverWalletSelection)

            WalletSettingsLoadError(
                message = state.message,
                title = stringResource(R.string.settings_wallet_load_settings_error_title),
                retryText = stringResource(R.string.action_try_again),
                backText = stringResource(R.string.action_go_back),
                onRetry = {
                    lastLoadFailureAlertMessage = null
                    loadAttempt++
                },
                onBack = recoverWalletSelection,
                modifier = modifier,
            )
        }

        is WalletSettingsLoadState.Ready -> {
            when (route) {
                WalletSettingsRoute.MAIN -> {
                    WalletSettingsScreen(
                        app = app,
                        manager = state.manager,
                        modifier = modifier,
                    )
                }
                WalletSettingsRoute.CHANGE_NAME -> {
                    WalletSettingsChangeNameScreen(
                        app = app,
                        manager = state.manager,
                        modifier = modifier,
                    )
                }
            }
        }
    }
}

private sealed interface WalletSettingsLoadState {
    data object Loading : WalletSettingsLoadState

    data class Ready(
        val manager: WalletManager,
    ) : WalletSettingsLoadState

    data class Failed(
        val message: String,
    ) : WalletSettingsLoadState

    data class Recovering(
        val message: String,
    ) : WalletSettingsLoadState
}

@Composable
private fun WalletSettingsLoadError(
    title: String,
    message: String,
    retryText: String,
    backText: String,
    onRetry: () -> Unit,
    onBack: () -> Unit,
    modifier: Modifier = Modifier,
) {
    Box(
        modifier = modifier.fillMaxSize(),
        contentAlignment = Alignment.Center,
    ) {
        Column(
            modifier = Modifier.padding(24.dp),
            horizontalAlignment = Alignment.CenterHorizontally,
            verticalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            Text(
                text = title,
                style = MaterialTheme.typography.bodyLarge,
                color = MaterialTheme.colorScheme.error,
                textAlign = TextAlign.Center,
            )
            Text(
                text = message,
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                textAlign = TextAlign.Center,
            )
            Button(onClick = onRetry) {
                Text(retryText)
            }
            TextButton(onClick = onBack) {
                Text(backText)
            }
        }
    }
}

private const val WALLET_LOAD_ERROR_RECOVERY_DELAY_MS = 5_000L
