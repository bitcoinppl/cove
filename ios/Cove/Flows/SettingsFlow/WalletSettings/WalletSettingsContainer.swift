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
    @State private var manager: WalletManager? = nil
    @State private var error: String? = nil

    func walletNameBinding(_ manager: WalletManager) -> Binding<String> {
        Binding(
            get: { manager.walletMetadata.name },
            set: { manager.dispatch(action: .updateName($0)) }
        )
    }

    func initOnAppear() {
        do {
            let manager = try app.getWalletManager(id: id)
            self.manager = manager
        } catch {
            self.error = "Failed to get wallet \(error.localizedDescription)"
            Log.error(self.error!)
        }
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

    @ViewBuilder
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
        .onAppear(perform: initOnAppear)
    }

    var body: some View {
        if let manager {
            WalletSettingsRoute(manager: manager, route: route)
        } else {
            LoadingOrError
        }
    }
}
