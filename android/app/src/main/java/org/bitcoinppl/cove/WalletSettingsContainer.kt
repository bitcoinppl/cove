package org.bitcoinppl.cove

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import org.bitcoinppl.cove_core.*
import org.bitcoinppl.cove_core.types.*

/**
 * wallet settings container - lazy loads WalletManager for wallet settings
 * ported from iOS WalletSettingsContainer.swift
 */
@Composable
fun WalletSettingsContainer(
    app: AppManager,
    id: WalletId,
    route: WalletSettingsRoute,
    modifier: Modifier = Modifier,
) {
    var manager by remember(id) { mutableStateOf<WalletManager?>(null) }
    val tag = "WalletSettingsContainer"

    // lazy load wallet manager
    LaunchedEffect(id) {
        try {
            android.util.Log.d(tag, "getting wallet $id")
            manager = app.getWalletManager(id)
        } catch (e: Exception) {
            android.util.Log.e(tag, "failed to load wallet", e)
            app.alertState =
                TaggedItem(
                    AppAlertState.General(
                        title = "Error!",
                        message = "Unable to load wallet: ${e.message}",
                    ),
                )
        }
    }

    // render
    when (val wm = manager) {
        null -> {
            Box(
                modifier = modifier.fillMaxSize(),
                contentAlignment = Alignment.Center,
            ) {
                CircularProgressIndicator()
            }
        }
        else -> {
            when (route) {
                WalletSettingsRoute.MAIN -> {
                    // TODO: implement WalletSettingsScreen with manager
                    Box(
                        modifier = modifier.fillMaxSize(),
                        contentAlignment = Alignment.Center,
                    ) {
                        androidx.compose.material3.Text("Wallet Settings - TODO")
                    }
                }
                WalletSettingsRoute.CHANGE_NAME -> {
                    // TODO: implement change wallet name screen
                    Box(
                        modifier = modifier.fillMaxSize(),
                        contentAlignment = Alignment.Center,
                    ) {
                        androidx.compose.material3.Text("Change Name - TODO")
                    }
                }
            }
        }
    }
}
