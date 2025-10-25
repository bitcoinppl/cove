package org.bitcoinppl.cove

import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.ui.Modifier
import org.bitcoinppl.cove.tapsigner.*
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

    // use current route or last in path
    val currentRoute = manager.path.lastOrNull() ?: manager.initialRoute

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
}
