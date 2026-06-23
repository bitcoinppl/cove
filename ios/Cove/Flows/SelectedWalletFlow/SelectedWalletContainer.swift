//
//  SelectedWalletContainer.swift
//  Cove
//
//  Created by Praveen Perera on 7/1/24.
//

import SwiftUI

struct SelectedWalletContainer: View {
    @Environment(\.colorScheme) private var colorScheme
    @Environment(AppManager.self) private var app

    let id: WalletId

    private var iOS26OrLater: Bool {
        if #available(iOS 26.0, *) { return true }
        return false
    }

    var body: some View {
        WalletManagerHost(
            walletId: id,
            loading: {
                FullPageLoadingView(title: "Loading wallet...")
            },
            onError: handleManagerError
        ) { manager in
            SelectedWalletScreen(manager: manager)
                .background(
                    iOS26OrLater
                        ? nil
                        : manager.loadState == .loading
                        ? LinearGradient(
                            colors: [
                                .black.opacity(colorScheme == .dark ? 0.9 : 0),
                                .black.opacity(colorScheme == .dark ? 0.9 : 0),
                            ], startPoint: .top, endPoint: .bottom
                        )
                        : LinearGradient(
                            stops: [
                                .init(
                                    color: .midnightBlue,
                                    location: 0.20
                                ),
                                .init(
                                    color: colorScheme == .dark ? .black.opacity(0.9) : .white,
                                    location: 0.20
                                ),
                            ], startPoint: .top, endPoint: .bottom
                        )
                )
                .background(iOS26OrLater ? nil : Color.white)
                .task {
                    // start scan immediately (sends cached data first, then scans)
                    do {
                        try await manager.rust.startWalletScan()
                    } catch {
                        Log.error("Wallet Scan Failed \(error.localizedDescription)")
                    }
                }
        }
    }

    private func handleManagerError(_ error: Error) {
        switch error {
        case let WalletManagerError.DatabaseCorruption(walletId, errorMessage):
            Log.error("Wallet database corrupted for \(walletId): \(errorMessage)")
            app.alertState = TaggedItem(
                .walletDatabaseCorrupted(walletId: walletId, error: errorMessage)
            )
        default:
            Log.error("Something went very wrong: \(error)")
            do {
                let wallets = try Database().wallets().all()
                let wallet = wallets.first(where: { $0.id != id })

                if let wallet {
                    try app.selectWalletOrThrow(wallet.id)
                } else {
                    app.loadAndReset(to: Route.newWallet(.select))
                }
            } catch {
                app.loadAndReset(to: Route.newWallet(.select))
            }
        }
    }
}

#Preview {
    SelectedWalletContainer(id: WalletId())
        .environment(AppManager.shared)
}
