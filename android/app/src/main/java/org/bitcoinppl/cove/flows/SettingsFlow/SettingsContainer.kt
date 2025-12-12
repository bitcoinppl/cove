package org.bitcoinppl.cove.flows.SettingsFlow

import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.material3.MaterialTheme
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.zIndex
import org.bitcoinppl.cove.R
import org.bitcoinppl.cove_core.*
import org.bitcoinppl.cove_core.types.*

/**
 * Settings container - lightweight router for settings screens
 * Ported from iOS SettingsContainer.swift
 */
@Composable
fun SettingsContainer(
    app: org.bitcoinppl.cove.AppManager,
    route: SettingsRoute,
    modifier: Modifier = Modifier,
) {
    Box(modifier = modifier.fillMaxSize()) {
        // background color overlay
        Box(
            modifier =
                Modifier
                    .fillMaxSize()
                    .background(MaterialTheme.colorScheme.background)
                    .zIndex(0f),
        )

        // background pattern (subtle Material Design texture)
        Image(
            painter = painterResource(id = R.drawable.image_chain_code_pattern_horizontal),
            contentDescription = null,
            contentScale = ContentScale.Crop,
            modifier =
                Modifier
                    .fillMaxSize()
                    .graphicsLayer(alpha = 0.08f)
                    .zIndex(1f),
        )

        // settings content
        Box(modifier = Modifier.fillMaxSize().zIndex(2f)) {
            when (route) {
                is SettingsRoute.Main -> {
                    MainSettingsScreen(
                        app = app,
                        modifier = modifier,
                    )
                }
                is SettingsRoute.Network -> {
                    NetworkSettingsScreen(
                        app = app,
                        modifier = modifier,
                    )
                }
                is SettingsRoute.Appearance -> {
                    AppearanceSettingsScreen(
                        app = app,
                        modifier = modifier,
                    )
                }
                is SettingsRoute.Node -> {
                    NodeSettingsScreen(
                        app = app,
                        modifier = modifier,
                    )
                }
                is SettingsRoute.FiatCurrency -> {
                    FiatCurrencySettingsScreen(
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
                    SettingsListAllWalletsScreen(
                        app = app,
                        modifier = modifier,
                    )
                }
            }
        }
    }
}
