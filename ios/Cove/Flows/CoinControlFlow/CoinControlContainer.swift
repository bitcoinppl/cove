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

    @State private var walletManager: WalletManager?
    @State private var manager: CoinControlManager?

    var body: some View {
        Group {
            if let walletManager, let manager {
                switch route {
                case .list:
                    UtxoListScreen(manager: manager)
                        .environment(walletManager)
                }
            } else {
                ProgressView()
                    .tint(.primary)
            }
        }
        .task(id: route.id()) {
            await loadManager()
        }
    }

    @MainActor
    private func loadManager() async {
        if walletManager != nil, manager != nil { return }

        do {
            Log.debug("Getting wallet for CoinControlRoute \(route.id())")
            let walletManager = try app.getWalletManager(id: route.id())
            let rustManager = try await walletManager.rust.newCoinControlManager()

            self.walletManager = walletManager
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
