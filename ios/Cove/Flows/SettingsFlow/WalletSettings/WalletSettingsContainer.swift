//
//  WalletSettingsContainer.swift
//  Cove
//
//  Created by Praveen Perera on 12/5/24.
//

import Foundation
import SwiftUI

struct WalletSettingsContainer: View {
    @Environment(AppManager.self) var app

    // args
    let id: WalletId
    let route: WalletSettingsRoute

    /// private
    @State private var error: String? = nil

    func walletNameBinding(_ manager: WalletManager) -> Binding<String> {
        Binding(
            get: { manager.walletMetadata.name },
            set: { manager.dispatch(action: .updateName($0)) }
        )
    }

    @ViewBuilder
    func WalletSettingsRoute(manager: WalletManager, route: WalletSettingsRoute) -> some View {
        switch route {
        case .main:
            WalletSettingsView(manager: manager)
        case .changeName:
            WalletSettingsChangeNameView(name: walletNameBinding(manager))
        }
    }

    var body: some View {
        WalletManagerHost(walletId: id, loading: {
            WalletSettingsLoadingOrError(error: error) {
                app.trySelectLatestOrNewWallet()
            }
        }, onError: { error in
            Log.error("Failed to get wallet settings: \(error.localizedDescription)")
            self.error = String(localized: "Unable to load wallet settings. Please try again.")
        }) { manager in
            WalletSettingsRoute(manager: manager, route: route)
        }
    }
}

private struct WalletSettingsLoadingOrError: View {
    let error: String?
    let recover: () -> Void

    var body: some View {
        Group {
            if let error {
                Text(error)
            } else {
                FullPageLoadingView()
            }
        }
        .task {
            guard let error else { return }
            Log.error(error)
            try? await Task.sleep(for: .seconds(5))
            recover()
        }
    }
}
