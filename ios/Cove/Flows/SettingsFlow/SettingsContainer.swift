//
//  SettingsContainer.swift
//  Cove
//
//  Created by Praveen Perera on 1/29/25.
//

import SwiftUI

struct SettingsContainer: View {
    @Environment(AppManager.self) private var app
    let route: SettingsRoute

    @State private var showNetworkChangeAlert = false
    @State private var pendingNetwork: Network? = nil

    var selectedNetwork: Binding<Network> {
        Binding(
            get: { app.selectedNetwork },
            set: { network in
                if network != app.selectedNetwork {
                    pendingNetwork = network
                    showNetworkChangeAlert = true
                }
            }
        )
    }

    @ViewBuilder
    var FiatCurrencyPicker: some View {
        SettingsPicker(selection:
            Binding(
                get: { app.selectedFiatCurrency },
                set: {
                    app.dispatch(action: .changeFiatCurrency($0))
                }
            )
        )
        .navigationTitle("Currency")
    }

    @ViewBuilder
    var AppearancePicker: some View {
        SettingsPicker(selection:
            Binding(
                get: { app.colorSchemeSelection },
                set: {
                    app.dispatch(action: .changeColorScheme($0))
                }
            )
        )
        .navigationTitle("Appearance")
    }

    var body: some View {
        Group {
            switch route {
            case .main:
                MainSettingsScreen()
            case .network:
                SettingsPicker(selection: selectedNetwork)
            case .appearance:
                AppearancePicker
            case .node:
                NodeSelectionView()
            case .fiatCurrency:
                FiatCurrencyPicker
            case let .wallet(id: walletId, route: route):
                WalletSettingsContainer(id: walletId, route: route)
            case .allWallets:
                SettingsListAllWallets()
            }
        }
        .navigationBarTitleDisplayMode(.inline)
        .background(
            ZStack {
                Color(UIColor.systemGroupedBackground)
                    .ignoresSafeArea(edges: .all)

                Image(.settingsPattern)
                    .resizable()
                    .aspectRatio(contentMode: .fill)
                    .frame(maxWidth: .infinity)
                    .ignoresSafeArea(edges: .all)
            }
        )
        .alert("Change Network?", isPresented: $showNetworkChangeAlert) {
            Button("Yes, Change Network") {
                if let network = pendingNetwork {
                    app.dispatch(action: .changeNetwork(network: network))
                    app.rust.selectLatestOrNewWallet()
                }
                pendingNetwork = nil
            }
            Button("Cancel", role: .cancel) {
                pendingNetwork = nil
            }
        } message: {
            if let network = pendingNetwork {
                Text("Switching to \(network) will take you to a wallet on that network.")
            }
        }
    }
}

#Preview {
    SettingsContainer(route: .main)
        .environment(AppManager.shared)
        .environment(AuthManager.shared)
}
