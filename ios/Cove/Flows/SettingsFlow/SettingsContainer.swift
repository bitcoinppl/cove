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

    var selectedNetwork: Binding<Network> {
        Binding(
            get: { app.selectedNetwork },
            set: { network in
                app.dispatch(action: .changeNetwork(network: network))
            }
        )
    }

    var FiatCurrencyPicker: SettingsPicker<FiatCurrency> {
        SettingsPicker(selection:
            Binding(
                get: { app.selectedFiatCurrency },
                set: {
                    app.dispatch(action: .changeFiatCurrency($0))
                }
            ))
    }

    var body: some View {
        Group {
            switch route {
            case .main:
                MainSettingsScreen()
            case .network:
                SettingsPicker(selection: selectedNetwork)
                    .navigationTitle("Network")
            case .appearance:
                EmptyView()
            case .node:
                EmptyView()
            case .fiatCurrency:
                FiatCurrencyPicker
                    .navigationTitle("Network")
            case .wallet(let walletId):
                WalletSettingsContainer(id: walletId)
            }
        }
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
    }
}

#Preview {
    SettingsContainer(route: .main)
        .environment(AppManager.shared)
        .environment(AuthManager.shared)
}
