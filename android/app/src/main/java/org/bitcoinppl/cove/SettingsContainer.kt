package org.bitcoinppl.cove

import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.zIndex
import org.bitcoinppl.cove.ui.theme.CoveColor
import org.bitcoinppl.cove_core.*
import org.bitcoinppl.cove_core.types.*

/**
 * settings container - lightweight router for settings screens
 * ported from iOS SettingsContainer.swift
 */
@Composable
fun SettingsContainer(
    app: AppManager,
    route: SettingsRoute,
    modifier: Modifier = Modifier,
) {
    Box(modifier = modifier.fillMaxSize()) {
        // background color overlay
        Box(
            modifier =
                Modifier
                    .fillMaxSize()
                    .background(CoveColor.ListBackgroundLight)
                    .zIndex(0f),
        )

        // background pattern
        Image(
            painter = painterResource(id = R.drawable.image_chain_code_pattern_horizontal),
            contentDescription = null,
            contentScale = ContentScale.Crop,
            modifier =
                Modifier
                    .fillMaxSize()
                    .graphicsLayer(alpha = 0.25f)
                    .zIndex(1f),
        )

        // settings content
        Box(modifier = Modifier.fillMaxSize().zIndex(2f)) {
            when (route) {
                is SettingsRoute.Main -> {
                    org.bitcoinppl.cove.settings.SettingsScreen(
                        app = app,
                        modifier = modifier,
                    )
                }
                is SettingsRoute.Network -> {
                    org.bitcoinppl.cove.settings.NetworkSettingsScreen(
                        app = app,
                        modifier = modifier,
                    )
                }
                is SettingsRoute.Appearance -> {
                    org.bitcoinppl.cove.settings.AppearanceSettingsScreen(
                        app = app,
                        modifier = modifier,
                    )
                }
                is SettingsRoute.Node -> {
                    org.bitcoinppl.cove.settings.NodeSettingsScreen(
                        app = app,
                        modifier = modifier,
                    )
                }
                is SettingsRoute.FiatCurrency -> {
                    org.bitcoinppl.cove.settings.FiatCurrencySettingsScreen(
                        app = app,
                        modifier = modifier,
                    )
                }
                is SettingsRoute.Wallet -> {
                    // wallet settings container (nested)
                    WalletSettingsContainer(
                        app = app,
                        id = route.id,
                        route = route.route,
                    )
                }
                is SettingsRoute.AllWallets -> {
                    org.bitcoinppl.cove.settings.SettingsListAllWalletsScreen(
                        app = app,
                        modifier = modifier,
                    )
                }
            }
        }
    }
}
