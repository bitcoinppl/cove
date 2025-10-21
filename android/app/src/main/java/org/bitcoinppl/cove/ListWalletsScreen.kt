package org.bitcoinppl.cove

import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.ui.Modifier
import org.bitcoinppl.cove.components.FullPageLoadingView
import org.bitcoinppl.cove_core.*
import org.bitcoinppl.cove_core.types.*

/**
 * list wallets screen - auto-selects first wallet or navigates to add wallet
 * ported from iOS ListWalletsScreen.swift
 */
@Composable
fun ListWalletsScreen(
    app: AppManager,
    modifier: Modifier = Modifier,
) {
    // show loading indicator
    FullPageLoadingView(modifier = modifier)

    // on appear, check for wallets and navigate
    LaunchedEffect(Unit) {
        try {
            val wallets = Database().wallets().all()
            android.util.Log.d("ListWalletsScreen", "Found ${wallets.size} wallets")

            val firstWallet = wallets.firstOrNull()
            if (firstWallet != null) {
                // select the first wallet
                app.rust.selectWallet(firstWallet.id)
            } else {
                // no wallets, go to add wallet screen
                app.loadAndReset(RouteFactory().newWalletSelect())
            }
        } catch (e: Exception) {
            android.util.Log.e("ListWalletsScreen", "Failed to get wallets", e)
            // on error, navigate to add wallet
            app.loadAndReset(RouteFactory().newWalletSelect())
        }
    }
}
