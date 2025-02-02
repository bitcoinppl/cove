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
    var AppearencePicker: some View {
        SettingsPicker(selection:
            Binding(
                get: { app.colorSchemeSelection },
                set: {
                    app.dispatch(action: .changeColorScheme($0))
                }
            )
        )
        .navigationTitle("Appearence")
    }

    var body: some View {
        Group {
            switch route {
            case .main:
                MainSettingsScreen()
            case .network:
                SettingsPicker(selection: selectedNetwork)
            case .appearance:
                AppearencePicker
            case .node:
                EmptyView()
            case .fiatCurrency:
                FiatCurrencyPicker
            case .wallet(let walletId):
                WalletSettingsContainer(id: walletId)
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
    }
}

#Preview {
    SettingsContainer(route: .main)
        .environment(AppManager.shared)
        .environment(AuthManager.shared)
}
