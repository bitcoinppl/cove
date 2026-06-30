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

    @State private var pendingNetwork: Network? = nil

    var selectedNetwork: Binding<Network> {
        Binding(
            get: { app.selectedNetwork },
            set: { network in
                if network != app.selectedNetwork {
                    pendingNetwork = network
                }
            }
        )
    }

    var body: some View {
        Group {
            switch route {
            case .main:
                MainSettingsScreen()
            case .network:
                SettingsPicker(selection: selectedNetwork)
            case .appearance:
                SettingsContainerPicker(
                    title: "Appearance",
                    selection: Binding(
                        get: { app.colorSchemeSelection },
                        set: {
                            app.dispatch(action: .changeColorScheme($0))
                        }
                    )
                )
            case .node:
                NodeSelectionView()
            case .blockExplorer:
                BlockExplorerSettingsView()
            case .fiatCurrency:
                SettingsContainerPicker(
                    title: "Currency",
                    selection: Binding(
                        get: { app.selectedFiatCurrency },
                        set: {
                            app.dispatch(action: .changeFiatCurrency($0))
                        }
                    )
                )
            case let .wallet(id: walletId, route: route):
                WalletSettingsContainer(id: walletId, route: route)
            case .allWallets:
                SettingsListAllWallets()
            case .about:
                AboutScreen()
            case .cloudBackup:
                CloudBackupDetailScreen()
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
        .alert("Change Network?", isPresented: Binding(
            get: { pendingNetwork != nil },
            set: { if !$0 { pendingNetwork = nil } }
        )) {
            Button("Yes, Change Network") {
                if let network = pendingNetwork {
                    app.dispatch(action: .changeNetwork(network: network))
                    app.trySelectLatestOrNewWallet()
                }
                pendingNetwork = nil
            }
            Button("Cancel", role: .cancel) {}
        } message: {
            if let network = pendingNetwork {
                Text("Switching to \(network) will take you to a wallet on that network.")
            }
        }
    }
}

private struct SettingsContainerPicker<T: SettingsEnum>: View where T.AllCases: RandomAccessCollection {
    let title: String
    @Binding var selection: T

    var body: some View {
        SettingsPicker(selection: $selection)
            .navigationTitle(title)
    }
}

#Preview {
    SettingsContainer(route: .main)
        .environment(AppManager.shared)
        .environment(AuthManager.shared)
        .environment(CloudBackupPresentationCoordinator())
}
