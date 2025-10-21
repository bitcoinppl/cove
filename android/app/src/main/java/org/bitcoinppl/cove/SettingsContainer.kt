package org.bitcoinppl.cove

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
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
        // TODO: add background pattern drawable (image_settings_pattern)
        // Image(
        //     painter = painterResource(id = R.drawable.image_settings_pattern),
        //     contentDescription = null,
        //     contentScale = ContentScale.FillBounds,
        //     modifier = Modifier
        //         .fillMaxSize()
        //         .zIndex(0f)
        // )

        // background color overlay
        Box(
            modifier =
                Modifier
                    .fillMaxSize()
                    .background(CoveColor.ListBackgroundLight)
                    .zIndex(0f),
        )

        // settings content
        Box(modifier = Modifier.fillMaxSize().zIndex(2f)) {
            when (route) {
                is SettingsRoute.Main -> {
                    // TODO: implement MainSettingsScreen with app parameter
                    androidx.compose.material3.Text("Main Settings - TODO")
                }
                is SettingsRoute.Network -> {
                    // TODO: implement network settings picker
                    androidx.compose.material3.Text("Network Settings - TODO")
                }
                is SettingsRoute.Appearance -> {
                    // TODO: implement appearance settings picker
                    androidx.compose.material3.Text("Appearance Settings - TODO")
                }
                is SettingsRoute.Node -> {
                    // TODO: implement node selection screen
                    androidx.compose.material3.Text("Node Settings - TODO")
                }
                is SettingsRoute.FiatCurrency -> {
                    // TODO: implement fiat currency picker
                    androidx.compose.material3.Text("Currency Settings - TODO")
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
                    // TODO: implement all wallets list
                    androidx.compose.material3.Text("All Wallets - TODO")
                }
            }
        }
    }
}
