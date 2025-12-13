package org.bitcoinppl.cove.flows.TapSignerFlow

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.remember
import androidx.compose.ui.Modifier
import org.bitcoinppl.cove.App
import org.bitcoinppl.cove_core.TapSignerRoute

/**
 * container for TapSigner flow
 * manages navigation between TapSigner screens
 * ported from iOS TapSignerContainer.swift
 */
@Composable
fun TapSignerContainer(
    route: TapSignerRoute,
    modifier: Modifier = Modifier,
) {
    val app = App
    val manager = remember(route) { TapSignerManager(route) }

    // cleanup on disappear
    DisposableEffect(route) {
        onDispose {
            manager.close()
        }
    }

    // use current route or last in path
    val currentRoute = manager.path.lastOrNull() ?: manager.initialRoute

    Box(modifier = modifier.fillMaxSize()) {
        when (currentRoute) {
            is TapSignerRoute.InitSelect -> {
                TapSignerChooseChainCode(
                    app = app,
                    manager = manager,
                    tapSigner = currentRoute.v1,
                    modifier = modifier.fillMaxSize(),
                )
            }

            is TapSignerRoute.InitAdvanced -> {
                TapSignerAdvancedChainCode(
                    app = app,
                    manager = manager,
                    tapSigner = currentRoute.v1,
                    modifier = modifier.fillMaxSize(),
                )
            }

            is TapSignerRoute.StartingPin -> {
                TapSignerStartingPinView(
                    app = app,
                    manager = manager,
                    tapSigner = currentRoute.tapSigner,
                    chainCode = currentRoute.chainCode,
                    modifier = modifier.fillMaxSize(),
                )
            }

            is TapSignerRoute.NewPin -> {
                TapSignerNewPinView(
                    app = app,
                    manager = manager,
                    args = currentRoute.v1,
                    modifier = modifier.fillMaxSize(),
                )
            }

            is TapSignerRoute.ConfirmPin -> {
                TapSignerConfirmPinView(
                    app = app,
                    manager = manager,
                    args = currentRoute.v1,
                    modifier = modifier.fillMaxSize(),
                )
            }

            is TapSignerRoute.SetupSuccess -> {
                TapSignerSetupSuccessView(
                    app = app,
                    manager = manager,
                    tapSigner = currentRoute.v1,
                    setup = currentRoute.v2,
                    modifier = modifier.fillMaxSize(),
                )
            }

            is TapSignerRoute.SetupRetry -> {
                TapSignerSetupRetryView(
                    app = app,
                    manager = manager,
                    tapSigner = currentRoute.v1,
                    response = currentRoute.v2,
                    modifier = modifier.fillMaxSize(),
                )
            }

            is TapSignerRoute.EnterPin -> {
                TapSignerEnterPinView(
                    app = app,
                    manager = manager,
                    tapSigner = currentRoute.tapSigner,
                    action = currentRoute.action,
                    modifier = modifier.fillMaxSize(),
                )
            }

            is TapSignerRoute.ImportSuccess -> {
                TapSignerImportSuccessView(
                    app = app,
                    manager = manager,
                    tapSigner = currentRoute.v1,
                    deriveInfo = currentRoute.v2,
                    modifier = modifier.fillMaxSize(),
                )
            }

            is TapSignerRoute.ImportRetry -> {
                TapSignerImportRetryView(
                    app = app,
                    manager = manager,
                    tapSigner = currentRoute.v1,
                    modifier = modifier.fillMaxSize(),
                )
            }
        }

        // show scanning overlay when NFC is active
        if (manager.isScanning) {
            TapSignerScanningOverlay(message = manager.scanMessage)
        }
    }
}
