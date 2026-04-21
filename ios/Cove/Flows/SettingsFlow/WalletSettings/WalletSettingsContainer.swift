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

    // private
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

    var LoadingOrError: some View {
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
            app.rust.selectLatestOrNewWallet()
        }
    }

    var body: some View {
        WalletManagerHost(walletId: id, loading: {
            LoadingOrError
        }, onError: { error in
            self.error = "Failed to get wallet \(error.localizedDescription)"
            Log.error(self.error!)
        }) { manager in
            WalletSettingsRoute(manager: manager, route: route)
        }
    }
}
