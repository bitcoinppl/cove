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

    private var colorSchemeSelection: Binding<ColorSchemeSelection> {
        Binding(
            get: { app.colorSchemeSelection },
            set: { app.dispatch(action: .changeColorScheme($0)) }
        )
    }

    private var selectedFiatCurrency: Binding<FiatCurrency> {
        Binding(
            get: { app.selectedFiatCurrency },
            set: { app.dispatch(action: .changeFiatCurrency($0)) }
        )
    }

    var body: some View {
        SettingsContainerChrome(
            pendingNetwork: $pendingNetwork,
            confirmNetworkChange: confirmNetworkChange
        ) {
            SettingsRouteContent(
                route: route,
                selectedNetwork: selectedNetwork,
                colorSchemeSelection: colorSchemeSelection,
                selectedFiatCurrency: selectedFiatCurrency
            )
        }
    }

    private func confirmNetworkChange() {
        guard let pendingNetwork else { return }

        app.dispatch(action: .changeNetwork(network: pendingNetwork))
        app.trySelectLatestOrNewWallet()
        self.pendingNetwork = nil
    }
}

private struct SettingsContainerChrome<Content: View>: View {
    @Binding var pendingNetwork: Network?
    let confirmNetworkChange: () -> Void
    @ViewBuilder let content: Content

    private var isNetworkChangePresented: Binding<Bool> {
        Binding(
            get: { pendingNetwork != nil },
            set: { if !$0 { pendingNetwork = nil } }
        )
    }

    var body: some View {
        content
            .navigationBarTitleDisplayMode(.inline)
            .background {
                ZStack {
                    Color(UIColor.systemGroupedBackground)
                        .ignoresSafeArea(edges: .all)
                    Image(.settingsPattern)
                        .resizable()
                        .aspectRatio(contentMode: .fill)
                        .frame(maxWidth: .infinity)
                        .ignoresSafeArea(edges: .all)
                }
            }
            .alert("Change Network?", isPresented: isNetworkChangePresented) {
                Button("Yes, Change Network", action: confirmNetworkChange)
                Button("Cancel", role: .cancel) {}
            } message: {
                if let pendingNetwork {
                    Text(verbatim: "Switching to \(pendingNetwork) will take you to a wallet on that network.")
                }
            }
    }
}

private struct SettingsRouteContent: View {
    private let content: AnyView

    init(
        route: SettingsRoute,
        selectedNetwork: Binding<Network>,
        colorSchemeSelection: Binding<ColorSchemeSelection>,
        selectedFiatCurrency: Binding<FiatCurrency>
    ) {
        content = switch route {
        case .main:
            AnyView(MainSettingsScreen())
        case .network:
            AnyView(SettingsPicker(selection: selectedNetwork))
        case .appearance:
            AnyView(SettingsContainerPicker(
                title: "Appearance",
                selection: colorSchemeSelection
            ))
        case .node:
            AnyView(NodeSelectionView())
        case .blockExplorer:
            AnyView(BlockExplorerSettingsView())
        case .fiatCurrency:
            AnyView(SettingsContainerPicker(
                title: "Currency",
                selection: selectedFiatCurrency
            ))
        case let .wallet(id: walletId, route: route):
            AnyView(WalletSettingsContainer(id: walletId, route: route))
        case .allWallets:
            AnyView(SettingsListAllWallets())
        case .about:
            AnyView(AboutScreen())
        case .cloudBackup:
            AnyView(CloudBackupDetailScreen())
        case .ohttpRelay:
            AnyView(OhttpRelaySettingsView())
        }
    }

    var body: some View {
        content
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
