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
            handleCoinControlManagerError(error)
        }
    }

    private func handleCoinControlManagerError(_ error: Error) {
        switch error {
        case WalletManagerError.InitialScanIncomplete:
            app.showInitialScanIncompleteAlert()
            app.popRoute()
        case let WalletManagerError.DatabaseCorruption(walletId, errorMessage):
            Log.error("Wallet database corrupted for \(walletId): \(errorMessage)")
            app.alertState = TaggedItem(
                .walletDatabaseCorrupted(walletId: walletId, error: errorMessage)
            )
            app.popRoute()
        case WalletManagerError.WalletDoesNotExist:
            Log.error("Wallet does not exist for coin control route \(route)")
            app.alertState = .init(.general(
                title: "Wallet Not Found",
                message: "This wallet is no longer available."
            ))
            app.trySelectLatestOrNewWallet()
        case let walletError as WalletManagerError:
            Log.error("Unable to open wallet for coin control: \(walletError)")
            app.alertState = .init(.general(
                title: "Unable to Open Wallet",
                message: "The wallet could not be opened for coin control. Please try again from the wallet screen."
            ))
            app.popRoute()
        default:
            Log.error("Unable to open wallet for coin control: \(error)")
            app.alertState = .init(.general(
                title: "Unable to Open Wallet",
                message: "The wallet could not be opened for coin control. Please try again from the wallet screen."
            ))
            app.popRoute()
        }
    }
}
