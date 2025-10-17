package org.bitcoinppl.cove

import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.zIndex
import com.example.cove.R

/**
 * settings container - lightweight router for settings screens
 * ported from iOS SettingsContainer.swift
 */
@Composable
fun SettingsContainer(
    app: AppManager,
    route: SettingsRoute,
    modifier: Modifier = Modifier
) {
    Box(modifier = modifier.fillMaxSize()) {
        // background pattern
        Image(
            painter = painterResource(id = R.drawable.image_settings_pattern),
            contentDescription = null,
            contentScale = ContentScale.FillBounds,
            modifier = Modifier
                .fillMaxSize()
                .zIndex(0f)
        )

        // background color overlay
        Box(
            modifier = Modifier
                .fillMaxSize()
                .background(Color(0xFFF2F2F7))
                .zIndex(1f)
        )

        // settings content
        Box(modifier = Modifier.fillMaxSize().zIndex(2f)) {
            when (route) {
                is SettingsRoute.Main -> {
                    org.bitcoinppl.cove.settings.SettingsScreen(
                        app = app
                    )
                }
                is SettingsRoute.Network -> {
                    org.bitcoinppl.cove.settings.NetworkSettingsScreen(
                        app = app
                    )
                }
                is SettingsRoute.Appearance -> {
                    org.bitcoinppl.cove.settings.AppearanceSettingsScreen(
                        app = app
                    )
                }
                is SettingsRoute.Node -> {
                    org.bitcoinppl.cove.settings.NodeSettingsScreen(
                        app = app
                    )
                }
                is SettingsRoute.FiatCurrency -> {
                    org.bitcoinppl.cove.settings.FiatCurrencySettingsScreen(
                        app = app
                    )
                }
                is SettingsRoute.Wallet -> {
                    // wallet settings container (nested)
                    WalletSettingsContainer(
                        app = app,
                        id = route.id,
                        route = route.route
                    )
                }
                is SettingsRoute.AllWallets -> {
                    // TODO: implement all wallets list
                    androidx.compose.material3.Text("All Wallets - TODO")
                }
            }
        }
    }
}
