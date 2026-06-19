//
//  CoinControlContainer.swift
//  Cove
//
//  Created by Praveen Perera on 5/19/25.
//

import Foundation
import SwiftUI

struct CoinControlContainer: View {
    @Environment(AppManager.self) private var app

    let route: CoinControlRoute

    var body: some View {
        WalletManagerHost(
            walletId: route.id(),
            loading: {
                ProgressView()
                    .tint(.primary)
            },
            onError: handleManagerError
        ) { walletManager in
            CoinControlLoadedView(route: route, walletManager: walletManager)
        }
    }

    private func handleManagerError(_ error: Error) {
        Log.error("[ERROR] Unable to get wallet \(error.localizedDescription)")
        app.alertState = .init(
            .general(
                title: "Error!", message: "Unable to get wallet \(error.localizedDescription)"
            )
        )
    }
}

private struct CoinControlLoadedView: View {
    @Environment(AppManager.self) private var app

    let route: CoinControlRoute
    let walletManager: WalletManager

    @State private var manager: CoinControlManager?

    var body: some View {
        Group {
            if let manager {
                switch route {
                case .list:
                    UtxoListScreen(manager: manager)
                }
            } else {
                ProgressView()
                    .tint(.primary)
            }
        }
        .task(id: ObjectIdentifier(walletManager)) {
            await loadManager()
        }
    }

    @MainActor
    private func loadManager() async {
        manager = nil
        do {
            let rustManager = try await walletManager.rust.newCoinControlManager()
            manager = CoinControlManager(rustManager)
        } catch {
            app.alertState = .init(.general(
                title: "Initial Scan Incomplete",
                message: "Can't send until initial scan completes."
            ))
            app.popRoute()
        }
    }
}
